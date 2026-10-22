using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Threading;
using FaceBridge.ModuleHost.Ipc;
using FaceBridge.ModuleHost.Loading;
using Microsoft.Extensions.Logging;
using VRCFaceTracking;
using VRCFaceTracking.Core.Library;
using VRCFaceTracking.Core.Params.Data;

namespace FaceBridge.ModuleHost;

/// <summary>
/// Drives one loaded module: <c>Initialize</c> then an <c>Update</c> loop on a dedicated thread, a frame after
/// each <c>Update</c> (at most one per millisecond) and <c>Status</c> polling.
/// </summary>
internal sealed class ModuleRunner
{
    private static readonly long MinFrameIntervalTicks = Stopwatch.Frequency / 1000;
    private const int JoinTimeoutMs = 2000;
    private int _tornDown;

    private readonly LoadedModule _loaded;
    private readonly IpcChannel _channel;
    private readonly Frame _frame = new();
    private Thread? _thread;
    private volatile bool _stopping;
    private ulong _seq;
    private sbyte _lastStatus = sbyte.MinValue;

    public ModuleRunner(LoadedModule loaded, IpcChannel channel)
    {
        _loaded = loaded;
        _channel = channel;
    }

    private ExtTrackingModule Module => _loaded.Instance;

    /// <summary>Starts initialization on the module thread; returns immediately. Ignored after the first call.</summary>
    public void Init(bool eyeAvailable, bool expressionAvailable)
    {
        if (_thread != null)
        {
            return;
        }

        _thread = new Thread(() => ThreadMain(eyeAvailable, expressionAvailable))
        {
            Name = "module-update",
            IsBackground = true,
        };
        _thread.Start();
    }

    /// <summary>
    /// Stops the update loop, waits for the in-flight <c>Update()</c> to return (up to 2 s) and only then calls the
    /// module's <c>Teardown()</c> once, so Teardown never overlaps Update. Does nothing if the module was never
    /// initialized. Blocks until done.
    /// </summary>
    public void Teardown()
    {
        _stopping = true;
        Thread? thread = _thread;
        if (thread == null || Interlocked.Exchange(ref _tornDown, 1) != 0)
        {
            return;
        }

        if (thread != Thread.CurrentThread && !thread.Join(JoinTimeoutMs))
        {
            _channel.HostLog(LogLevel.Warning, "Update() was still running 2 s after the stop request; calling Teardown() anyway.");
        }

        try
        {
            Module.Teardown();
        }
        catch (Exception e)
        {
            _channel.HostLog(LogLevel.Error, "Module Teardown threw.", e);
        }
    }

    private void ThreadMain(bool eyeAvailable, bool expressionAvailable)
    {
        try
        {
            (bool eyeClaimed, bool exprClaimed) = Module.Initialize(eyeAvailable, expressionAvailable);
            if (_stopping)
            {
                return;
            }

            bool eye = eyeClaimed && eyeAvailable;
            bool expression = exprClaimed && expressionAvailable;
            Module.ModuleInformation.UsingEye = eye;
            Module.ModuleInformation.UsingExpression = expression;

            _channel.Send(new InitResult(eye, expression, Module.ModuleInformation.Name ?? string.Empty, ReadImages()));
            PollStatus();
            if (!eye && !expression)
            {
                return;
            }

            FrameFlags claimFlags = (eye ? FrameFlags.Eye : 0) | (expression ? FrameFlags.Expression : 0);
            long lastFrame = 0;
            bool anyFrame = false;
            while (!_stopping)
            {
                Module.Update();
                PollStatus();

                // Every Update ends in a frame so the module's final state is never lost if the next Update
                // blocks for a while; frames are at least 1 ms apart, so wait out the remainder of the window.
                if (anyFrame)
                {
                    var spinner = new SpinWait();
                    while (!_stopping && Stopwatch.GetTimestamp() - lastFrame < MinFrameIntervalTicks)
                    {
                        spinner.SpinOnce(sleep1Threshold: -1);
                    }
                }

                if (!_stopping && FillFrame(claimFlags))
                {
                    _channel.Send(_frame);
                }

                lastFrame = Stopwatch.GetTimestamp();
                anyFrame = true;
            }
        }
        catch (Exception e) when (!_stopping)
        {
            _channel.HostLog(LogLevel.Critical, $"Unhandled exception in {_loaded.Type.FullName}.", e);
            Program.Exit(ExitCodes.ModuleCrashed);
        }
        catch (Exception)
        {
            // Raised by the module while it was being torn down; expected.
        }
    }

    private void PollStatus()
    {
        sbyte state = (sbyte)Module.Status;
        if (state != _lastStatus)
        {
            _lastStatus = state;
            _channel.Send(new Status(state));
        }
    }

    private byte[][] ReadImages()
    {
        List<Stream>? streams = Module.ModuleInformation.StaticImages;
        if (streams == null || streams.Count == 0)
        {
            return Array.Empty<byte[]>();
        }

        var images = new List<byte[]>(Math.Min(streams.Count, ushort.MaxValue));
        foreach (Stream? stream in streams)
        {
            if (stream == null || images.Count == ushort.MaxValue)
            {
                continue;
            }

            try
            {
                long position = stream.CanSeek ? stream.Position : 0;
                if (stream.CanSeek)
                {
                    stream.Position = 0;
                }

                using var copy = new MemoryStream();
                stream.CopyTo(copy);
                if (stream.CanSeek)
                {
                    stream.Position = position;
                }

                images.Add(copy.ToArray());
            }
            catch (Exception e) when (e is IOException or NotSupportedException or ObjectDisposedException)
            {
                _channel.HostLog(LogLevel.Warning, "A module static image could not be read.", e);
            }
        }

        return images.ToArray();
    }

    /// <summary>Copies the static <c>UnifiedTracking.Data</c> into the reusable frame; false if the module nulled it.</summary>
    private bool FillFrame(FrameFlags claimFlags)
    {
        UnifiedTrackingData? data = UnifiedTracking.Data;
        if (data?.Eye == null || data.Shapes == null)
        {
            return false;
        }

        UnifiedEyeData eye = data.Eye;
        UnifiedHeadData head = data.Head;
        Frame f = _frame;
        f.Seq = ++_seq;
        f.LeftGazeX = eye.Left.Gaze.x;
        f.LeftGazeY = eye.Left.Gaze.y;
        f.LeftOpenness = eye.Left.Openness;
        f.LeftPupilMm = eye.Left.PupilDiameter_MM;
        f.RightGazeX = eye.Right.Gaze.x;
        f.RightGazeY = eye.Right.Gaze.y;
        f.RightOpenness = eye.Right.Openness;
        f.RightPupilMm = eye.Right.PupilDiameter_MM;
        f.MaxDilation = eye._maxDilation;
        f.MinDilation = eye._minDilation;
        f.LeftDiameter = eye._leftDiameter;
        f.RightDiameter = eye._rightDiameter;
        f.HeadYaw = head.HeadYaw;
        f.HeadPitch = head.HeadPitch;
        f.HeadRoll = head.HeadRoll;
        f.HeadPosX = head.HeadPosX;
        f.HeadPosY = head.HeadPosY;
        f.HeadPosZ = head.HeadPosZ;

        UnifiedExpressionShape[] shapes = data.Shapes;
        int count = Math.Min(shapes.Length, ushort.MaxValue);
        if (f.Shapes.Length != count)
        {
            f.Shapes = new float[count];
        }

        for (int i = 0; i < count; i++)
        {
            f.Shapes[i] = shapes[i].Weight;
        }

        f.ShapeCount = count;

        bool anyHead = head.HeadYaw != 0 || head.HeadPitch != 0 || head.HeadRoll != 0
            || head.HeadPosX != 0 || head.HeadPosY != 0 || head.HeadPosZ != 0;
        f.Flags = claimFlags | (anyHead ? FrameFlags.Head : 0);
        return true;
    }
}
