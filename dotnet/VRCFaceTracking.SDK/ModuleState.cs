namespace VRCFaceTracking.Core.Library;

/// <summary>Lifecycle state a module reports through <see cref="VRCFaceTracking.ExtTrackingModule.Status"/>.</summary>
public enum ModuleState
{
    Uninitialized = -1,
    Idle = 0,
    Active = 1,
}
