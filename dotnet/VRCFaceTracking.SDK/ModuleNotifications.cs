using Microsoft.Extensions.Logging;

namespace VRCFaceTracking;

/// <summary>
/// User notifications are log entries carrying a well-known <see cref="Microsoft.Extensions.Logging.EventId"/>.
/// </summary>
public static class ModuleNotifications
{
    public const int Id = 7400;
    public const string Name = "VRCFT.UserNotification";

    public static readonly EventId EventId = new(Id, Name);

    public static void NotifyUser(ILogger logger, LogLevel level, string message)
    {
        logger.Log(level, EventId, message, null, static (state, _) => state);
    }

    public static bool IsNotification(EventId eventId) => eventId.Id == Id || eventId.Name == Name;
}
