using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;

namespace FaceBridge.Sdk.Hosting;

/// <summary>What the FaceBridge module host provides to <c>FaceBridge.Sdk</c>. Implemented by the host only.</summary>
[EditorBrowsable(EditorBrowsableState.Never)]
public interface IModuleHostBridge
{
    /// <summary>Show an informational pop-up; <paramref name="key"/> is empty when the module gave none.</summary>
    void Notify(LogLevel level, string key, string message);

    /// <summary>Show a message that stays until dismissed or cleared; <paramref name="key"/> may be empty.</summary>
    void NotifyMessage(LogLevel level, string key, string message);

    /// <summary>Remove the message with <paramref name="key"/>; empty removes all of this module's.</summary>
    void ClearNotifications(string key);

    /// <summary>The module declared its settings; <paramref name="schemaJson"/> follows the FaceBridge schema.</summary>
    void PublishSettingsSchema(string schemaJson);

    /// <summary>The module changed setting values itself; <paramref name="valuesJson"/> is an object of key to value.</summary>
    void WriteSettingValues(string valuesJson);

    /// <summary>A log line from <c>FaceBridge.Sdk</c> itself (e.g. a button handler threw).</summary>
    void Log(LogLevel level, string message, Exception? exception);
}

/// <summary>
/// Registration point the FaceBridge module host uses to connect this assembly to itself. Module authors never
/// call this; it is public only so the host (a different assembly) can.
/// </summary>
[EditorBrowsable(EditorBrowsableState.Never)]
public static class ModuleHostRegistration
{
    private static readonly object Gate = new();
    private static readonly Dictionary<string, JsonElement> PendingValues = new(StringComparer.Ordinal);
    private static ModuleSettings? _attached;

    internal static IModuleHostBridge? Bridge { get; private set; }

    /// <summary>Called once by the host before the module is instantiated.</summary>
    public static void Register(IModuleHostBridge bridge)
    {
        ArgumentNullException.ThrowIfNull(bridge);
        Bridge = bridge;
    }

    /// <summary>
    /// Called by the host with the current setting values (object of key to value), before and after the module
    /// declared its settings. Change callbacks run on the calling thread.
    /// </summary>
    public static void DeliverSettings(string valuesJson)
    {
        ModuleSettings? target;
        lock (Gate)
        {
            Merge(valuesJson);
            target = _attached;
        }

        target?.ApplyJson(valuesJson, raiseChanged: true);
    }

    /// <summary>
    /// Called by the host when the user pressed the settings button <paramref name="key"/>. Returns at once; the
    /// module's handler runs on a thread-pool thread. Unknown keys, and actions before the module declared its
    /// settings, are ignored.
    /// </summary>
    public static void DeliverAction(string key)
    {
        ArgumentNullException.ThrowIfNull(key);
        ModuleSettings? target;
        lock (Gate)
        {
            target = _attached;
        }

        _ = DispatchAction(target, key, Bridge);
    }

    /// <summary>
    /// Runs button <paramref name="key"/> of <paramref name="target"/> on the thread pool; a throwing handler is
    /// logged through <paramref name="bridge"/> as an error. The task never faults.
    /// </summary>
    internal static Task DispatchAction(ModuleSettings? target, string key, IModuleHostBridge? bridge)
    {
        Action? handler = target?.ButtonHandler(key);
        if (handler == null)
        {
            bridge?.Log(
                LogLevel.Debug,
                target == null ? $"Ignoring settings action '{key}': the module has not declared settings." : $"Ignoring settings action '{key}': no such button.",
                null);
            return Task.CompletedTask;
        }

        return Task.Run(() =>
        {
            try
            {
                handler();
            }
            catch (Exception e)
            {
                bridge?.Log(LogLevel.Error, $"Settings button '{key}' handler threw.", e);
            }
        });
    }

    /// <summary>Connects a declared <see cref="ModuleSettings"/> to the host; returns values the host delivered earlier.</summary>
    internal static string Attach(ModuleSettings settings)
    {
        lock (Gate)
        {
            _attached = settings;
            var ms = new System.IO.MemoryStream();
            using (var writer = new Utf8JsonWriter(ms))
            {
                writer.WriteStartObject();
                foreach (KeyValuePair<string, JsonElement> pair in PendingValues)
                {
                    writer.WritePropertyName(pair.Key);
                    pair.Value.WriteTo(writer);
                }

                writer.WriteEndObject();
            }

            return System.Text.Encoding.UTF8.GetString(ms.GetBuffer(), 0, (int)ms.Length);
        }
    }

    private static void Merge(string valuesJson)
    {
        using JsonDocument doc = JsonDocument.Parse(valuesJson);
        if (doc.RootElement.ValueKind != JsonValueKind.Object)
        {
            throw new ArgumentException("Settings values must be a JSON object.", nameof(valuesJson));
        }

        foreach (JsonProperty property in doc.RootElement.EnumerateObject())
        {
            PendingValues[property.Name] = property.Value.Clone();
        }
    }
}
