using System;
using System.Diagnostics;
using System.IO;
using System.Net.Sockets;
using System.Threading;
using FaceBridge.ModuleHost.Ipc;
using FaceBridge.ModuleHost.Loading;
using FaceBridge.ModuleHost.Logging;
using FaceBridge.Sdk.Hosting;
using Microsoft.Extensions.Logging;

namespace FaceBridge.ModuleHost;

internal static class Program
{
    private static int Main(string[] args)
    {
        if (args.Length == 2 && args[0] == "--probe")
        {
            if (!Directory.Exists(args[1]))
            {
                Console.Error.WriteLine($"Probe directory not found: {args[1]}");
                return ExitCodes.BadArguments;
            }

            return Probe.Run(args[1], Console.Out);
        }

        Options? options = Options.Parse(args, out string error);
        if (options == null)
        {
            Console.Error.WriteLine(error);
            return ExitCodes.BadArguments;
        }

        int code = Run(options);
        Exit(code);
        return code;
    }

    /// <summary>
    /// Ends the process immediately, without waiting for foreground threads the module may have started. Before
    /// that the connection is closed gracefully so the host reads the last messages (LoadFailed, TeardownDone,
    /// crash logs) instead of seeing a reset.
    /// </summary>
    internal static void Exit(int code)
    {
        if (code != ExitCodes.ParentGone)
        {
            ShutdownConnection();
        }

        Environment.Exit(code);
    }

    private static Socket? _socket;

    private static void ShutdownConnection()
    {
        Socket? socket = _socket;
        if (socket == null)
        {
            return;
        }

        try
        {
            socket.Shutdown(SocketShutdown.Send);
            var waited = Stopwatch.StartNew();
            while (waited.ElapsedMilliseconds < 500)
            {
                if (!socket.Poll(50_000, SelectMode.SelectRead))
                {
                    continue;
                }

                if (socket.Available == 0)
                {
                    break; // host closed its side
                }

                Thread.Sleep(5); // unread data from the host; the read loop may still be consuming it
            }
        }
        catch (Exception e) when (e is SocketException or ObjectDisposedException)
        {
            // Already closed.
        }
    }

    private static int Run(Options options)
    {
        IpcChannel channel;
        Stream stream;
        try
        {
            var client = new TcpClient { NoDelay = true };
            client.Connect(options.Endpoint);
            stream = client.GetStream();
            _socket = client.Client;
            channel = new IpcChannel(stream);
            if (!channel.Send(new Hello(Protocol.Version, options.Token, (uint)Environment.ProcessId)))
            {
                return ExitCodes.ConnectFailed;
            }
        }
        catch (Exception e) when (e is SocketException or IOException)
        {
            Console.Error.WriteLine($"Cannot connect to host {options.Endpoint}: {e.Message}");
            return ExitCodes.ConnectFailed;
        }

        ConsoleRedirect.Install(channel);
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
        {
            channel.HostLog(LogLevel.Critical, "Unhandled exception.", e.ExceptionObject as Exception);
            Exit(ExitCodes.ModuleCrashed);
        };
        StartParentWatchdog(options.ParentPid);
        SetEnvironment(options);
        ModuleHostRegistration.Register(new HostBridge(channel));

        LoadedModule loaded;
        bool supportsEye, supportsExpression;
        try
        {
            loaded = LoadedModule.Load(options.ModulePath, type => new IpcLogger(channel, type.FullName ?? type.Name));
            (supportsEye, supportsExpression) = loaded.Instance.Supported;
        }
        catch (Exception e)
        {
            channel.HostLog(LogLevel.Error, "Module load failed.", e);
            channel.Send(new LoadFailed(e.Message));
            return ExitCodes.LoadFailed;
        }

        channel.Send(new Loaded(loaded.Type.FullName ?? loaded.Type.Name, supportsEye, supportsExpression));
        return ReadLoop(stream, channel, new ModuleRunner(loaded, channel));
    }

    private static int ReadLoop(Stream stream, IpcChannel channel, ModuleRunner runner)
    {
        byte[] scratch = new byte[4096];
        while (true)
        {
            Message? message;
            try
            {
                message = Protocol.ReadMessage(stream, ref scratch);
            }
            catch (Exception e) when (e is ProtocolException or IOException or ObjectDisposedException)
            {
                Console.Error.WriteLine($"Connection to host failed: {e.Message}");
                runner.Teardown();
                return e is ProtocolException ? ExitCodes.ConnectFailed : ExitCodes.Ok;
            }

            switch (message)
            {
                case null:
                    runner.Teardown();
                    return ExitCodes.Ok;
                case Settings settings:
                    try
                    {
                        ModuleHostRegistration.DeliverSettings(settings.Json);
                    }
                    catch (Exception e)
                    {
                        channel.HostLog(LogLevel.Error, "Applying settings failed.", e);
                    }

                    break;
                case SettingsAction action:
                    // Returns at once: the module's handler runs on the thread pool, never on this reader thread.
                    ModuleHostRegistration.DeliverAction(action.Key);
                    break;
                case Init init:
                    runner.Init(init.EyeAvailable, init.ExpressionAvailable);
                    break;
                case Teardown:
                    runner.Teardown();
                    channel.Send(new TeardownDone());
                    return ExitCodes.Ok;
                default:
                    channel.HostLog(LogLevel.Warning, $"Ignoring unexpected message {message.Type}.");
                    break;
            }
        }
    }

    private static void SetEnvironment(Options options)
    {
        // Read by VRCFaceTracking.Utils in the compat assembly when it first loads.
        string moduleDir = Path.GetDirectoryName(options.ModulePath)!;
        string? modulesDir = Path.GetDirectoryName(moduleDir);
        Environment.SetEnvironmentVariable("FACEBRIDGE_MODULE_DATA_DIR", options.DataDir);
        if (modulesDir != null)
        {
            Environment.SetEnvironmentVariable("FACEBRIDGE_MODULES_DIR", modulesDir);
        }
    }

    private static void StartParentWatchdog(int parentPid)
    {
        Process parent;
        try
        {
            parent = Process.GetProcessById(parentPid);
        }
        catch (ArgumentException)
        {
            Exit(ExitCodes.ParentGone);
            return;
        }

        new Thread(() =>
        {
            try
            {
                parent.WaitForExit();
            }
            catch (Exception)
            {
                // Cannot watch the handle any more; treat as gone.
            }

            Exit(ExitCodes.ParentGone);
        })
        {
            Name = "parent-watchdog",
            IsBackground = true,
        }.Start();
    }
}
