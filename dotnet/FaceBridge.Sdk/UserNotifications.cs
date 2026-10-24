using System;
using FaceBridge.Sdk.Hosting;
using Microsoft.Extensions.Logging;

namespace FaceBridge.Sdk;

/// <summary>
/// Shows things to the user. In FaceBridge, <see cref="Show"/> is an informational pop-up that disappears by itself,
/// and <see cref="ShowMessage"/> is a message that also stays on the Home page until the user dismisses it or the
/// module calls <see cref="ClearMessage"/>. Elsewhere both are log entries with
/// <c>EventId(7400, "VRCFT.UserNotification")</c>: VRCFaceTracking builds that support module notifications show them
/// to the user, others only log them.
/// </summary>
public static class UserNotifications
{
    public const int EventIdValue = 7400;
    public const string EventName = "VRCFT.UserNotification";

    private static readonly EventId NotificationEventId = new(EventIdValue, EventName);

    /// <summary>Informational pop-up; nothing is kept once it fades.</summary>
    /// <param name="logger">The module's <c>Logger</c>; used when not running in FaceBridge.</param>
    /// <param name="key">Optional. Ignored by FaceBridge for pop-ups; kept for symmetry with <see cref="ShowMessage"/>.</param>
    public static void Show(ILogger? logger, LogLevel level, string message, string? key = null)
    {
        ArgumentNullException.ThrowIfNull(message);
        IModuleHostBridge? bridge = ModuleHostRegistration.Bridge;
        if (bridge != null)
        {
            bridge.Notify(level, key ?? string.Empty, message);
            return;
        }

        LogFallback(logger, level, message);
    }

    /// <summary>
    /// A message the user has to act on: a pop-up plus an entry on the Home page that stays until dismissed or
    /// cleared.
    /// </summary>
    /// <param name="logger">The module's <c>Logger</c>; used when not running in FaceBridge.</param>
    /// <param name="key">Optional. A new message with the same key replaces the old one; without a key, the same
    /// text is not repeated.</param>
    public static void ShowMessage(ILogger? logger, LogLevel level, string message, string? key = null)
    {
        ArgumentNullException.ThrowIfNull(message);
        IModuleHostBridge? bridge = ModuleHostRegistration.Bridge;
        if (bridge != null)
        {
            bridge.NotifyMessage(level, key ?? string.Empty, message);
            return;
        }

        LogFallback(logger, level, message);
    }

    /// <summary>
    /// Removes this module's message with <paramref name="key"/>, or all of them when no key is given.
    /// Does nothing outside FaceBridge (VRCFaceTracking has no way to retract a notification).
    /// </summary>
    public static void ClearMessage(string? key = null)
    {
        ModuleHostRegistration.Bridge?.ClearNotifications(key ?? string.Empty);
    }

    private static void LogFallback(ILogger? logger, LogLevel level, string message) =>
        logger?.Log(level, NotificationEventId, message, null, static (state, _) => state);
}
