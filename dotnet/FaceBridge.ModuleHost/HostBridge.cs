using System;
using FaceBridge.ModuleHost.Ipc;
using FaceBridge.Sdk.Hosting;
using Microsoft.Extensions.Logging;

namespace FaceBridge.ModuleHost;

/// <summary>Connects <c>FaceBridge.Sdk</c> calls made by the module to IPC messages.</summary>
internal sealed class HostBridge : IModuleHostBridge
{
    private readonly IpcChannel _channel;

    public HostBridge(IpcChannel channel)
    {
        _channel = channel;
    }

    public void Notify(LogLevel level, string key, string message)
    {
        // Same as the logger path: notifications are also log lines.
        _channel.Send(new Log(level, 7400, "FaceBridge.Sdk", message, string.Empty));
        _channel.Send(new Ipc.Notify(level, key, message));
    }

    public void ClearNotifications(string key) => _channel.Send(new NotifyClear(key));

    public void PublishSettingsSchema(string schemaJson) => _channel.Send(new SettingsSchema(schemaJson));

    public void WriteSettingValues(string valuesJson) => _channel.Send(new SettingsWrite(valuesJson));

    public void Log(LogLevel level, string message, Exception? exception) => _channel.HostLog(level, message, exception);
}
