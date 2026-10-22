using System;
using VRCFaceTracking.Core.Params.Data;
using VRCFaceTracking.Core.Types;

namespace VRCFaceTracking;

/// <summary>Static data channel modules write their results into.</summary>
public class UnifiedTracking
{
    public static Image EyeImageData = new();
    public static Image LipImageData = new();
    public static UnifiedTrackingData Data = new();
    public static Action<UnifiedTrackingData> OnUnifiedDataUpdated;

    public UnifiedTracking()
    {
    }
}
