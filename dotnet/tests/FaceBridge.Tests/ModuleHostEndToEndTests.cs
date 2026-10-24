using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text.Json.Nodes;
using FaceBridge.ModuleHost.Ipc;
using Microsoft.Extensions.Logging;
using Xunit;

namespace FaceBridge.Tests;

/// <summary>Launches the built FaceBridge.ModuleHost.exe against the built TestModule with an in-process fake Rust host.</summary>
public class ModuleHostEndToEndTests
{
    private static string PrepareModule()
    {
        string dir = Path.Combine(TestPaths.NewTempDir("e2e"), "modules", "test-module");
        TestPaths.CopyDirectory(TestPaths.ProjectOutput("TestModule"), dir);
        return dir;
    }

    private static (string module, string data) Paths()
    {
        string dir = PrepareModule();
        return (Path.Combine(dir, "TestModule.dll"), TestPaths.NewTempDir("data"));
    }

    [Fact]
    public void Full_session_hello_loaded_init_frames_notify_settings_teardown()
    {
        (string module, string data) = Paths();
        using var host = new FakeHost();
        host.Start(module, data);
        host.Accept();

        Hello hello = host.Expect<Hello>();
        Assert.Equal(1, hello.Version);
        Assert.Equal(FakeHost.Token, hello.Token);
        Assert.Equal((uint)host.Process!.Id, hello.Pid);

        // Stored values go out before Init, as the protocol says.
        host.Send(new Settings("""{ "gain": 8, "bogus": true }"""));
        Loaded loaded = host.Expect<Loaded>();
        Assert.Equal("TestModule.TestTrackingModule", loaded.ModuleType);
        Assert.True(loaded.SupportsEye);
        Assert.True(loaded.SupportsExpression);

        host.Send(new Init(true, true));
        InitResult init = host.Expect<InitResult>();
        Assert.True(init.Eye);
        Assert.True(init.Expression);
        Assert.Equal("Test Module", init.Name);
        Assert.Equal(new byte[] { 1, 2, 3 }, Assert.Single(init.Images));

        // Settings declared through FaceBridge.Sdk arrive as a schema.
        JsonNode schema = JsonNode.Parse(host.Expect<SettingsSchema>().Json)!;
        JsonArray fields = schema["fields"]!.AsArray();
        Assert.Equal(new[] { "gain", "mode", "pair_code", "pair", "pair_status" }, fields.Select(f => (string)f!["key"]!).ToArray());
        Assert.Equal("int", (string?)fields[0]!["type"]);
        Assert.Equal(5, (int)fields[0]!["default"]!);
        Assert.Equal(10, (int)fields[0]!["max"]!);
        Assert.Equal("Scales the jaw output.", (string?)fields[0]!["description"]);
        Assert.Equal("manual", (string?)fields[1]!["options"]![1]!["value"]);

        // Notifications: via FaceBridge.Sdk (keyed) and via a plain event-7400 log entry (keyless).
        Notify keyed = host.Expect<Notify>(n => n.Key == "no-headset");
        Assert.Equal(LogLevel.Warning, keyed.Level);
        Assert.Equal("No headset answered", keyed.Message);
        Notify plain = host.Expect<Notify>(n => n.Message == "plain notice");
        Assert.Equal(string.Empty, plain.Key);
        Assert.Equal(LogLevel.Information, plain.Level);

        // The module's console output and logger land in Log.
        host.Expect<Log>(l => l.Category == "Console.Out" && l.Message == "console says hi");
        host.Expect<Log>(l => l.Category == "TestModule.TestTrackingModule" && l.Message == "isFaceBridge=True");
        Assert.Equal(1, host.Expect<Status>().State);

        // The module writes a setting back.
        JsonNode write = JsonNode.Parse(host.Expect<SettingsWrite>().Json)!;
        Assert.Equal("manual", (string?)write["mode"]);

        // Frames: sequence starts at 1 and counts up; values come from UnifiedTracking.Data; gain 8 applied from stored settings.
        Frame first = host.Expect<Frame>();
        Assert.Equal(1UL, first.Seq);
        Assert.Equal(FrameFlags.Eye | FrameFlags.Expression, first.Flags);
        Assert.Equal(0.25f, first.LeftGazeX);
        Assert.Equal(-0.5f, first.LeftGazeY);
        Assert.Equal(0.75f, first.LeftOpenness);
        Assert.Equal(4f, first.RightPupilMm);
        Assert.Equal(88, first.ShapeCount);
        Assert.Equal(0.8f, first.Shapes[22], 5);
        Frame second = host.Expect<Frame>();
        Assert.Equal(2UL, second.Seq);

        // A later user change reaches the module's Changed callback and the next frames.
        host.Send(new Settings("""{ "gain": 3 }"""));
        host.Expect<Log>(l => l.Message == "setting gain changed to 3");
        Frame changed = host.Expect<Frame>(f => Math.Abs(f.Shapes[22] - 0.3f) < 1e-5f);
        Assert.True(changed.Seq > second.Seq);

        host.Send(new Teardown());
        host.Expect<TeardownDone>();
        Assert.Equal(0, host.WaitForExit());
    }

    [Fact]
    public void Settings_button_runs_the_module_handler_which_updates_text_and_notifies()
    {
        using FakeHost host = StartInitialized(string.Empty);
        JsonArray fields = JsonNode.Parse(host.Expect<SettingsSchema>().Json)!["fields"]!.AsArray();
        JsonNode button = fields.Single(f => (string?)f!["key"] == "pair")!;
        Assert.Equal("button", (string?)button["type"]);
        Assert.Equal("Pair by code", (string?)button["label"]);
        Assert.Null(button["default"]);
        JsonNode status = fields.Single(f => (string?)f!["key"] == "pair_status")!;
        Assert.Equal("text", (string?)status["type"]);
        Assert.Equal("Not paired", (string?)status["default"]);

        // No code yet: the handler updates the status text and warns.
        host.Send(new SettingsAction("pair"));
        host.Expect<SettingsWrite>(w => (string?)JsonNode.Parse(w.Json)!["pair_status"] == "Enter a code first");
        NotifyMessage warning = host.Expect<NotifyMessage>(n => n.Key == "pair");
        Assert.Equal(LogLevel.Warning, warning.Level);
        Assert.Equal("Enter a pairing code before pressing Pair.", warning.Message);

        // A stale status value from the host is ignored; the code the user typed is used.
        host.Send(new Settings("""{ "pair_code": "1234", "pair_status": "stale" }"""));
        host.Send(new SettingsAction("pair"));
        SettingsWrite paired = host.Expect<SettingsWrite>(w => w.Json.Contains("Paired with code"));
        Assert.Equal("""{"pair_status":"Paired with code 1234"}""", paired.Json);
        host.Expect<NotifyClear>(c => c.Key == "pair");

        // Unknown buttons are ignored; the module keeps running.
        host.Send(new SettingsAction("bogus"));
        host.Send(new Teardown());
        host.Expect<TeardownDone>();
        Assert.Equal(0, host.WaitForExit());
        Assert.DoesNotContain(host.All, m => m is Log { Level: LogLevel.Error });
    }

    [Fact]
    public void Module_that_throws_in_Update_is_logged_and_exits_with_code_5()
    {
        (string module, string data) = Paths();
        using var host = new FakeHost();
        host.Start(module, data, env: new Dictionary<string, string> { ["FB_TEST_SCENARIO"] = "crash" });
        host.Accept();
        host.Expect<Hello>();
        host.Expect<Loaded>();
        host.Send(new Init(true, true));

        Log crash = host.Expect<Log>(l => l.Level == LogLevel.Critical);
        Assert.Contains("boom from Update", crash.Exception);
        Assert.Equal(5, host.WaitForExit());
    }

    private static FakeHost StartInitialized(string scenario, params (string, string)[] extraEnv)
    {
        (string module, string data) = Paths();
        var env = new Dictionary<string, string> { ["FB_TEST_SCENARIO"] = scenario };
        foreach ((string k, string v) in extraEnv)
        {
            env[k] = v;
        }

        var host = new FakeHost();
        host.Start(module, data, env: env);
        host.Accept();
        host.Expect<Loaded>();
        host.Send(new Init(true, true));
        host.Expect<InitResult>();
        return host;
    }

    [Fact]
    public void Teardown_waits_for_the_in_flight_Update_before_calling_module_Teardown()
    {
        using FakeHost host = StartInitialized("slow", ("FB_TEST_SLOW_MS", "600"));
        host.Expect<Frame>(); // the next Update (600 ms) starts right after this frame

        var watch = Stopwatch.StartNew();
        host.Send(new Teardown());
        host.Expect<Log>(l => l.Message == "teardown overlapped update=False");
        host.Expect<TeardownDone>();
        Assert.Equal(0, host.WaitForExit());
        Assert.DoesNotContain(host.All, m => m is Log l && l.Message.Contains("still running"));
        Assert.True(watch.ElapsedMilliseconds >= 200, "Teardown should have waited for the running Update to finish");
    }

    [Fact]
    public void Teardown_still_runs_after_2_seconds_if_Update_does_not_return()
    {
        using FakeHost host = StartInitialized("slow", ("FB_TEST_SLOW_MS", "4000"));
        host.Expect<Frame>();

        host.Send(new Teardown());
        host.Expect<Log>(l => l.Level == LogLevel.Warning && l.Message.Contains("still running"));
        host.Expect<Log>(l => l.Message == "teardown overlapped update=True");
        host.Expect<TeardownDone>();
        Assert.Equal(0, host.WaitForExit());
    }

    [Fact]
    public void Final_state_of_an_Update_that_returns_within_1ms_is_sent_before_the_module_blocks()
    {
        using FakeHost host = StartInitialized("burst");
        // Update #3 blocks for 10 s; the state written by Update #2 must reach the host regardless.
        Frame last = host.Expect<Frame>(f => Math.Abs(f.Shapes[22] - 0.7f) < 1e-5f, TimeSpan.FromSeconds(5));
        Assert.True(last.Seq >= 2);
    }

    [Fact]
    public void Missing_module_file_gives_LoadFailed_and_exit_code_4()
    {
        using var host = new FakeHost();
        host.Start(Path.Combine(TestPaths.NewTempDir("none"), "Missing.dll"), TestPaths.NewTempDir("data"));
        host.Accept();
        host.Expect<Hello>();
        LoadFailed failed = host.Expect<LoadFailed>();
        Assert.Contains("Missing.dll", failed.Reason);
        Assert.Equal(4, host.WaitForExit());
    }

    [Fact]
    public void Dll_without_a_module_type_gives_LoadFailed_and_exit_code_4()
    {
        string dir = TestPaths.NewTempDir("notmodule");
        File.Copy(Path.Combine(TestPaths.ProjectOutput("FaceBridge.Sdk"), "FaceBridge.Sdk.dll"), Path.Combine(dir, "FaceBridge.Sdk.dll"));
        using var host = new FakeHost();
        host.Start(Path.Combine(dir, "FaceBridge.Sdk.dll"), TestPaths.NewTempDir("data"));
        host.Accept();
        host.Expect<Hello>();
        Assert.Contains("no public non-abstract ExtTrackingModule", host.Expect<LoadFailed>().Reason);
        Assert.Equal(4, host.WaitForExit());
    }

    [Fact]
    public void Closing_the_connection_ends_the_module_host_with_exit_code_0()
    {
        (string module, string data) = Paths();
        using var host = new FakeHost();
        host.Start(module, data);
        host.Accept();
        host.Expect<Loaded>();
        host.CloseConnection();
        Assert.Equal(0, host.WaitForExit());
    }

    [Fact]
    public void Unreachable_host_gives_exit_code_3()
    {
        (string module, string data) = Paths();
        // Port 1 on loopback has no listener.
        using var host = new FakeHost();
        host.Start(module, data, port: 1);
        Assert.Equal(3, host.WaitForExit());
    }

    [Theory]
    [InlineData("--module")]
    [InlineData("--bogus x")]
    [InlineData("--connect 127.0.0.1:5 --token t --module relative.dll --data-dir C:\\x --parent-pid 1")]
    public void Bad_arguments_give_exit_code_2(string args)
    {
        var psi = new ProcessStartInfo(TestPaths.ModuleHostExe, args) { UseShellExecute = false, RedirectStandardError = true, CreateNoWindow = true };
        psi.Environment["DOTNET_ROOT"] = TestPaths.DotnetRoot;
        using Process p = Process.Start(psi)!;
        Assert.True(p.WaitForExit(30_000));
        Assert.Equal(2, p.ExitCode);
        Assert.NotEmpty(p.StandardError.ReadToEnd());
    }

    [Fact]
    public void Exits_with_code_6_when_the_parent_process_dies()
    {
        (string module, string data) = Paths();
        var parentInfo = OperatingSystem.IsWindows()
            ? new ProcessStartInfo("ping.exe", "-n 60 127.0.0.1")
            : new ProcessStartInfo("sleep", "60");
        parentInfo.UseShellExecute = false;
        parentInfo.CreateNoWindow = true;
        parentInfo.RedirectStandardOutput = true;
        using Process parent = Process.Start(parentInfo)!;
        using var host = new FakeHost();
        host.Start(module, data, parentPid: parent.Id);
        host.Accept();
        host.Expect<Loaded>();

        parent.Kill();
        Assert.Equal(6, host.WaitForExit());
    }

    [Fact]
    public void Probe_mode_prints_module_type_and_exits_0()
    {
        string dir = PrepareModule();
        var psi = new ProcessStartInfo(TestPaths.ModuleHostExe) { UseShellExecute = false, RedirectStandardOutput = true, CreateNoWindow = true };
        psi.ArgumentList.Add("--probe");
        psi.ArgumentList.Add(dir);
        psi.Environment["DOTNET_ROOT"] = TestPaths.DotnetRoot;
        using Process p = Process.Start(psi)!;
        string output = p.StandardOutput.ReadToEnd();
        Assert.True(p.WaitForExit(30_000));
        Assert.Equal(0, p.ExitCode);
        Assert.Equal("{\"modules\":[{\"dll\":\"TestModule.dll\",\"type\":\"TestModule.TestTrackingModule\"}]}", output.Trim());
    }
}
