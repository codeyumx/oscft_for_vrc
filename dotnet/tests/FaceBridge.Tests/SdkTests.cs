using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.Loader;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using FaceBridge.Sdk;
using FaceBridge.Sdk.Hosting;
using Microsoft.Extensions.Logging;
using Xunit;

namespace FaceBridge.Tests;

/// <summary>FaceBridge.Sdk behaviour outside FaceBridge (no host registered in this process).</summary>
public class SdkTests
{
    /// <summary>A copy of TestModule.dll in a temp folder, so ModuleSettings stores its JSON file there.</summary>
    private static Assembly ModuleAssemblyInTempDir(out string directory)
    {
        directory = TestPaths.NewTempDir("sdk");
        TestPaths.CopyDirectory(TestPaths.ProjectOutput("TestModule"), directory);
        return new AssemblyLoadContext(null, isCollectible: true).LoadFromAssemblyPath(Path.Combine(directory, "TestModule.dll"));
    }

    private static ModuleSettings SpecSettings(Assembly asm) => new ModuleSettings(asm)
        .AddInt("port", "UDP port", 11112, 1, 65535, "Port the headset sends to.")
        .AddFloat("smoothing", "Smoothing", 0.5f, 0f, 1f, 0.05f)
        .AddBool("verbose", "Verbose logging", false)
        .AddString("host", "Headset address", string.Empty)
        .AddChoice("mode", "Mode", "auto", new[] { new ChoiceOption("auto", "Automatic"), new ChoiceOption("manual", "Manual") });

    [Fact]
    public void Schema_json_matches_the_spec_example()
    {
        Assembly asm = ModuleAssemblyInTempDir(out _);
        const string expected = """
        {
          "fields": [
            { "key": "port", "label": "UDP port", "description": "Port the headset sends to.", "type": "int",
              "default": 11112, "min": 1, "max": 65535 },
            { "key": "smoothing", "label": "Smoothing", "type": "float", "default": 0.5, "min": 0, "max": 1, "step": 0.05 },
            { "key": "verbose", "label": "Verbose logging", "type": "bool", "default": false },
            { "key": "host", "label": "Headset address", "type": "string", "default": "" },
            { "key": "mode", "label": "Mode", "type": "choice", "default": "auto",
              "options": [ { "value": "auto", "label": "Automatic" }, { "value": "manual", "label": "Manual" } ] }
          ]
        }
        """;
        JsonNode actual = JsonNode.Parse(SpecSettings(asm).BuildSchemaJson())!;
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse(expected), actual), actual.ToJsonString());
    }

    [Fact]
    public void Defaults_apply_then_values_persist_next_to_the_module_and_reload()
    {
        Assembly asm = ModuleAssemblyInTempDir(out string dir);
        ModuleSettings first = SpecSettings(asm);
        first.Declare();
        Assert.Equal(11112, first.GetInt("port"));
        Assert.Equal(0.5f, first.GetFloat("smoothing"));
        Assert.False(first.GetBool("verbose"));
        Assert.Equal("auto", first.GetString("mode"));

        first.Set("port", 5000);
        first.Set("smoothing", 2.5); // clamped to max
        first.Set("verbose", true);
        first.Set("host", "10.0.0.2");
        first.Set("mode", "manual");
        Assert.Equal(1f, first.GetFloat("smoothing"));
        Assert.True(File.Exists(Path.Combine(dir, "TestModule.settings.json")));

        ModuleSettings second = SpecSettings(asm);
        second.Declare();
        Assert.Equal(5000, second.GetInt("port"));
        Assert.Equal(1f, second.GetFloat("smoothing"));
        Assert.True(second.GetBool("verbose"));
        Assert.Equal("10.0.0.2", second.GetString("host"));
        Assert.Equal("manual", second.GetString("mode"));
    }

    [Fact]
    public void Corrupt_settings_file_falls_back_to_defaults()
    {
        Assembly asm = ModuleAssemblyInTempDir(out string dir);
        File.WriteAllText(Path.Combine(dir, "TestModule.settings.json"), "{ not json");
        ModuleSettings s = SpecSettings(asm);
        s.Declare();
        Assert.Equal(11112, s.GetInt("port"));
    }

    [Fact]
    public void Set_validates_type_range_and_key()
    {
        ModuleSettings s = SpecSettings(ModuleAssemblyInTempDir(out _));
        Assert.Throws<ArgumentException>(() => s.Set("verbose", "yes"));
        Assert.Throws<ArgumentException>(() => s.Set("mode", "nonexistent"));
        Assert.Throws<KeyNotFoundException>(() => s.Set("nope", 1));
        Assert.Throws<InvalidOperationException>(() => s.GetFloat("port"));
        Assert.Throws<ArgumentException>(() => s.AddBool("port", "dup", true));
        s.Set("port", 99999);
        Assert.Equal(65535, s.GetInt("port"));
    }

    [Fact]
    public void Host_delivered_values_raise_Changed_only_for_real_changes_and_ignore_invalid_ones()
    {
        ModuleSettings s = SpecSettings(ModuleAssemblyInTempDir(out _));
        var changes = new List<(string, object)>();
        s.Changed += (_, e) => changes.Add((e.Key, e.Value));

        s.ApplyJson("""{ "port": 22, "smoothing": 0.25, "verbose": "maybe", "mode": "bogus", "unknown": 1, "host": "" }""", raiseChanged: true);

        Assert.Equal(new (string, object)[] { ("port", 22), ("smoothing", 0.25f) }, changes);
        Assert.Equal(22, s.GetInt("port"));
        Assert.False(s.GetBool("verbose"));
        Assert.Equal("auto", s.GetString("mode"));

        changes.Clear();
        s.ApplyJson("""{ "port": 22.0, "smoothing": 5 }""", raiseChanged: true); // same int, clamped float
        Assert.Equal(new (string, object)[] { ("smoothing", 1f) }, changes);
    }

    [Fact]
    public void Notifications_outside_FaceBridge_are_log_entries_with_event_7400()
    {
        Assert.False(FaceBridgeEnvironment.IsFaceBridge);
        var logger = new CapturingLogger();
        UserNotifications.Show(logger, LogLevel.Warning, "Check your headset", key: "hs");
        UserNotifications.ClearMessage("hs"); // no-op, must not throw

        (LogLevel level, EventId id, string message) = Assert.Single(logger.Entries);
        Assert.Equal(LogLevel.Warning, level);
        Assert.Equal(7400, id.Id);
        Assert.Equal("VRCFT.UserNotification", id.Name);
        Assert.Equal("Check your headset", message);
    }

    private static ModuleSettings PairingSettings(Assembly asm, Action? onPair = null) => new ModuleSettings(asm)
        .AddString("code", "Pairing code", string.Empty)
        .AddButton("pair", "Pair by code", onPair ?? (() => { }), "Pairs the headset.")
        .AddText("status", "Status", "Not paired");

    [Fact]
    public void Schema_has_text_with_its_current_text_as_default_and_button_without_default()
    {
        ModuleSettings s = PairingSettings(ModuleAssemblyInTempDir(out _));
        s.SetText("status", "Waiting for headset…\nline 2");
        const string expected = """
        {
          "fields": [
            { "key": "code", "label": "Pairing code", "type": "string", "default": "" },
            { "key": "pair", "label": "Pair by code", "description": "Pairs the headset.", "type": "button" },
            { "key": "status", "label": "Status", "type": "text", "default": "Waiting for headset…\nline 2" }
          ]
        }
        """;
        JsonNode actual = JsonNode.Parse(s.BuildSchemaJson())!;
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse(expected), actual), actual.ToJsonString());
        Assert.Equal("Waiting for headset…\nline 2", s.GetString("status"));
    }

    [Fact]
    public void Host_delivered_values_do_not_change_text_or_buttons()
    {
        ModuleSettings s = PairingSettings(ModuleAssemblyInTempDir(out _));
        var changes = new List<string>();
        s.Changed += (_, e) => changes.Add(e.Key);

        s.ApplyJson("""{ "status": "stale", "pair": "x", "code": "1234" }""", raiseChanged: true);

        Assert.Equal(new[] { "code" }, changes);
        Assert.Equal("Not paired", s.GetString("status"));
        Assert.Equal("1234", s.GetString("code"));
    }

    [Fact]
    public void Buttons_have_no_value_and_SetText_needs_a_text_field()
    {
        ModuleSettings s = PairingSettings(ModuleAssemblyInTempDir(out _));
        Assert.Throws<InvalidOperationException>(() => s.Set("pair", "x"));
        Assert.Throws<InvalidOperationException>(() => s.Set("pair", true));
        Assert.Throws<InvalidOperationException>(() => s.GetString("pair"));
        Assert.Throws<InvalidOperationException>(() => s.GetBool("pair"));
        Assert.Throws<InvalidOperationException>(() => s.GetInt("pair"));
        Assert.Throws<InvalidOperationException>(() => s.SetText("pair", "x"));
        Assert.Throws<InvalidOperationException>(() => s.SetText("code", "x"));
        Assert.Throws<KeyNotFoundException>(() => s.SetText("nope", "x"));
        Assert.Throws<ArgumentException>(() => s.Set("status", 5));
        Assert.Throws<ArgumentNullException>(() => s.AddButton("b", "B", null!));

        s.Set("status", "via Set");
        Assert.Equal("via Set", s.GetString("status"));
    }

    [Fact]
    public void Text_and_buttons_are_not_saved_to_or_read_from_the_settings_file()
    {
        Assembly asm = ModuleAssemblyInTempDir(out string dir);
        string file = Path.Combine(dir, "TestModule.settings.json");
        ModuleSettings first = PairingSettings(asm);
        first.Declare();
        first.SetText("status", "Paired");
        Assert.False(File.Exists(file)); // text is runtime state: setting it writes nothing

        first.Set("code", "1234");
        JsonNode saved = JsonNode.Parse(File.ReadAllText(file))!;
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""{ "code": "1234" }"""), saved), saved.ToJsonString());

        File.WriteAllText(file, """{ "code": "5678", "status": "from file", "pair": 1 }""");
        ModuleSettings second = PairingSettings(asm);
        second.Declare();
        Assert.Equal("5678", second.GetString("code"));
        Assert.Equal("Not paired", second.GetString("status"));
    }

    [Fact]
    public async Task Actions_run_the_matching_button_handler_on_the_thread_pool()
    {
        int pairs = 0, resets = 0;
        bool onPool = false;
        ModuleSettings s = PairingSettings(ModuleAssemblyInTempDir(out _), () =>
        {
            onPool = Thread.CurrentThread.IsThreadPoolThread;
            Interlocked.Increment(ref pairs);
        });
        s.AddButton("reset", "Reset", () => Interlocked.Increment(ref resets));
        var bridge = new RecordingBridge();

        await ModuleHostRegistration.DispatchAction(s, "pair", bridge);
        Assert.Equal(1, pairs);
        Assert.Equal(0, resets);
        Assert.True(onPool);

        await ModuleHostRegistration.DispatchAction(s, "reset", bridge);
        Assert.Equal(1, resets);

        // Unknown keys, non-button keys and actions before settings were declared are ignored.
        await ModuleHostRegistration.DispatchAction(s, "nope", bridge);
        await ModuleHostRegistration.DispatchAction(s, "status", bridge);
        await ModuleHostRegistration.DispatchAction(null, "pair", bridge);
        ModuleHostRegistration.DeliverAction("pair"); // nothing attached in this process
        Assert.Equal(1, pairs);
        Assert.All(bridge.Logs, l => Assert.True(l.Level <= LogLevel.Information));
    }

    [Fact]
    public async Task A_throwing_button_handler_is_logged_as_an_error_and_contained()
    {
        ModuleSettings s = PairingSettings(ModuleAssemblyInTempDir(out _), () => throw new InvalidOperationException("pairing exploded"));
        var bridge = new RecordingBridge();

        await ModuleHostRegistration.DispatchAction(s, "pair", bridge); // must not fault

        (LogLevel level, string message, Exception? exception) = Assert.Single(bridge.Logs);
        Assert.Equal(LogLevel.Error, level);
        Assert.Contains("pair", message);
        Assert.Equal("pairing exploded", exception!.Message);
    }

    private sealed class RecordingBridge : IModuleHostBridge
    {
        private readonly object _gate = new();
        private readonly List<(LogLevel Level, string Message, Exception? Exception)> _logs = new();

        public List<(LogLevel Level, string Message, Exception? Exception)> Logs
        {
            get
            {
                lock (_gate)
                {
                    return new(_logs);
                }
            }
        }

        public void Notify(LogLevel level, string key, string message)
        {
        }

        public void NotifyMessage(LogLevel level, string key, string message)
        {
        }

        public void ClearNotifications(string key)
        {
        }

        public void PublishSettingsSchema(string schemaJson)
        {
        }

        public void WriteSettingValues(string valuesJson)
        {
        }

        public void Log(LogLevel level, string message, Exception? exception)
        {
            lock (_gate)
            {
                _logs.Add((level, message, exception));
            }
        }
    }

    private sealed class CapturingLogger : ILogger
    {
        public List<(LogLevel, EventId, string)> Entries { get; } = new();

        public IDisposable? BeginScope<TState>(TState state)
            where TState : notnull => null;

        public bool IsEnabled(LogLevel logLevel) => true;

        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            Entries.Add((logLevel, eventId, formatter(state, exception)));
        }
    }
}
