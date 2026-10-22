using System;
using FaceBridge.ModuleHost.Ipc;
using Microsoft.Extensions.Logging;

namespace FaceBridge.ModuleHost.Logging;

/// <summary>
/// <see cref="ILogger"/> handed to the module. Everything becomes an IPC <c>Log</c>; entries with event id 7400 /
/// name <c>VRCFT.UserNotification</c> additionally become a <c>Notify</c>.
/// </summary>
internal sealed class IpcLogger : ILogger
{
    private const int NotificationId = 7400;
    private const string NotificationName = "VRCFT.UserNotification";

    private readonly IpcChannel _channel;
    private readonly string _category;

    public IpcLogger(IpcChannel channel, string category)
    {
        _channel = channel;
        _category = category;
    }

    public IDisposable? BeginScope<TState>(TState state)
        where TState : notnull
    {
        return null;
    }

    public bool IsEnabled(LogLevel logLevel) => logLevel != LogLevel.None;

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
    {
        if (logLevel == LogLevel.None)
        {
            return;
        }

        string message;
        try
        {
            message = formatter(state, exception) ?? string.Empty;
        }
        catch (Exception e)
        {
            message = $"(log formatter threw {e.GetType().Name}: {e.Message}) {state}";
        }

        _channel.Send(new Ipc.Log(logLevel, eventId.Id, _category, message, exception?.ToString() ?? string.Empty));
        if (eventId.Id == NotificationId || eventId.Name == NotificationName)
        {
            _channel.Send(new Notify(logLevel, string.Empty, message));
        }
    }
}
