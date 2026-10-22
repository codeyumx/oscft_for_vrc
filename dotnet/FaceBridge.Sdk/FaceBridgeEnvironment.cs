using FaceBridge.Sdk.Hosting;

namespace FaceBridge.Sdk;

/// <summary>Tells a module which application is running it.</summary>
public static class FaceBridgeEnvironment
{
    /// <summary>
    /// <see langword="true"/> when the module runs inside the FaceBridge module host; <see langword="false"/> in
    /// VRCFaceTracking (or anything else).
    /// </summary>
    public static bool IsFaceBridge => ModuleHostRegistration.Bridge != null;
}
