using VRCFaceTracking.Core.Types;

namespace VRCFaceTracking.Core.Params.Data;

public struct UnifiedSingleEyeData
{
    /// <summary>Normalized gaze direction components (x right, y up).</summary>
    public Vector2 Gaze;

    public float PupilDiameter_MM;

    /// <summary>0 closed .. 1 open.</summary>
    public float Openness;

    public UnifiedSingleEyeData()
    {
        Gaze = Vector2.zero;
        PupilDiameter_MM = 0f;
        Openness = 0f;
    }
}
