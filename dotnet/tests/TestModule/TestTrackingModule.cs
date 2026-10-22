using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using FaceBridge.Sdk;
using Microsoft.Extensions.Logging;
using VRCFaceTracking;
using VRCFaceTracking.Core.Library;
using VRCFaceTracking.Core.Params.Expressions;
using VRCFaceTracking.Core.Types;

namespace TestModule;

public sealed class TestTrackingModule : ExtTrackingModule
{
    private ModuleSettings _settings = null!;
    private bool _wroteBack;
    private volatile bool _inUpdate;
    private int _calls;

    public override (bool SupportsEye, bool SupportsExpression) Supported => (true, true);

    public override (bool eyeSuccess, bool expressionSuccess) Initialize(bool eyeAvailable, bool expressionAvailable)
    {
        ModuleInformation.Name = "Test Module";
        ModuleInformation.StaticImages = new List<Stream> { new MemoryStream(new byte[] { 1, 2, 3 }) };

        _settings = new ModuleSettings()
            .AddInt("gain", "Gain", 5, min: 1, max: 10, description: "Scales the jaw output.")
            .AddChoice("mode", "Mode", "auto", new[] { new ChoiceOption("auto", "Automatic"), new ChoiceOption("manual", "Manual") })
            .AddString("pair_code", "Pairing code", string.Empty)
            .AddButton("pair", "Pair by code", Pair)
            .AddText("pair_status", "Status", "Not paired");
        _settings.Changed += (_, e) => Logger.LogInformation("setting {Key} changed to {Value}", e.Key, e.Value);
        _settings.Declare();

        if (Environment.GetEnvironmentVariable("FACEBRIDGE_DEMO_ERROR") == "1")
        {
            Logger.LogError("Demo error: port 11112 udp is not allowed by the firewall");
        }

        UserNotifications.Show(Logger, LogLevel.Warning, "No headset answered", "no-headset");
        Logger.Log(LogLevel.Information, new EventId(7400, "VRCFT.UserNotification"), "plain notice");
        Console.WriteLine("console says hi");
        Logger.LogInformation("isFaceBridge={IsFaceBridge}", FaceBridgeEnvironment.IsFaceBridge);

        Status = ModuleState.Active;
        return (eyeAvailable, expressionAvailable);
    }

    /// <summary>The "Pair by code" button.</summary>
    private void Pair()
    {
        string code = _settings.GetString("pair_code").Trim();
        if (code.Length == 0)
        {
            _settings.SetText("pair_status", "Enter a code first");
            UserNotifications.Show(Logger, LogLevel.Warning, "Enter a pairing code before pressing Pair.", key: "pair");
            return;
        }

        _settings.SetText("pair_status", $"Paired with code {code}");
        UserNotifications.Clear("pair");
    }

    public override void Update()
    {
        string scenario = Environment.GetEnvironmentVariable("FB_TEST_SCENARIO") ?? string.Empty;
        if (scenario == "crash")
        {
            throw new InvalidOperationException("boom from Update");
        }

        _inUpdate = true;
        try
        {
            if (!_wroteBack)
            {
                _wroteBack = true;
                _settings.Set("mode", "manual");
            }

            var data = UnifiedTracking.Data;
            data.Eye.Left.Openness = 0.75f;
            data.Eye.Left.Gaze = new Vector2(0.25f, -0.5f);
            data.Eye.Right.PupilDiameter_MM = 4f;
            data.Shapes[(int)UnifiedExpressions.JawOpen].Weight = _settings.GetInt("gain") / 10f;

            switch (scenario)
            {
                case "slow": // a long Update that is in flight when Teardown arrives
                    Thread.Sleep(int.Parse(Environment.GetEnvironmentVariable("FB_TEST_SLOW_MS") ?? "400"));
                    break;
                case "burst": // two instant Updates, then a long block: the second state must still be sent
                    int call = Interlocked.Increment(ref _calls);
                    if (call == 1)
                    {
                        data.Shapes[(int)UnifiedExpressions.JawOpen].Weight = 0.1f;
                    }
                    else if (call == 2)
                    {
                        data.Shapes[(int)UnifiedExpressions.JawOpen].Weight = 0.7f;
                    }
                    else
                    {
                        Thread.Sleep(10_000);
                    }

                    break;
                default:
                    Thread.Sleep(5);
                    break;
            }
        }
        finally
        {
            _inUpdate = false;
        }
    }

    public override void Teardown()
    {
        Logger.LogInformation("teardown overlapped update={Overlap}", _inUpdate);
        Status = ModuleState.Idle;
    }
}
