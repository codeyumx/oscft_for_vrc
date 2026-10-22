using System;
using FaceBridge.Sdk.Hosting;
using Microsoft.Extensions.Logging;

namespace FaceBridge.Sdk;

/// <summary>
/// Shows messages to the user. In FaceBridge they appear in the notification area (keyed, so a new message with the
/// same key replaces the old one). Elsewhere they are log entries with <c>EventId(7400, "VRCFT.UserNotification")</c>:
/// VRCFaceTracking builds that support module notifications show them to the user, others only log them.
/// </summary>
public static class UserNotifications
{
    public const int EventIdValue = 7400;
    public const string EventName = "VRCFT.UserNotification";

    private static readonly EventId NotificationEventId = new(EventIdValue, EventName);

    /// <param name="logger">The module's <c>Logger</c>; used when not running in FaceBridge.</param>
    /// <param name="key">Optional. Messages with the same key replace each other in FaceBridge.</param>
    public static void Show(ILogger? logger, LogLevel level, string message, string? key = null)
    {
        ArgumentNullException.ThrowIfNull(message);
        IModuleHostBridge? bridge = ModuleHostRegistration.Bridge;
        if (bridge != null)
        {
            bridge.Notify(level, key ?? string.Empty, message);
            return;
        }

        logger?.Log(level, NotificationEventId, message, null, static (state, _) => state);
    }

    /// <summary>
    /// Removes this module's notification with <paramref name="key"/>, or all of them when no key is given.
    /// Does nothing outside FaceBridge (VRCFaceTracking has no way to retract a notification).
    /// </summary>
    public static void Clear(string? key = null)
    {
        ModuleHostRegistration.Bridge?.ClearNotifications(key ?? string.Empty);
    }
}
