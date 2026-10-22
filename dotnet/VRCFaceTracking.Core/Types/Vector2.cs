using System;

namespace VRCFaceTracking.Core.Types;

public struct Vector2
{
    public float x;
    public float y;

    public static Vector2 zero => new(0f, 0f);

    public Vector2(float x, float y)
    {
        this.x = x;
        this.y = y;
    }

    /// <summary>
    /// Treats this vector as (azimuth, elevation) in radians and returns the XY projection of a ray of length
    /// <paramref name="r"/> pointing in that direction (x right, y up).
    /// </summary>
    public Vector2 PolarTo2DCartesian(float r)
    {
        return new Vector2(
            r * MathF.Sin(x) * MathF.Cos(y),
            r * MathF.Sin(y));
    }

    public Vector2 FlipXCoordinates() => new(-x, y);

    /// <summary>
    /// Treats this vector as the XY components of a unit gaze direction and returns the yaw in radians
    /// (positive to the right). Inverse of <see cref="PolarTo2DCartesian"/> for r = 1.
    /// </summary>
    public float ToYaw()
    {
        float z = MathF.Sqrt(MathF.Max(0f, 1f - x * x - y * y));
        return MathF.Atan2(x, z);
    }

    /// <summary>Pitch in radians (positive up) of the unit gaze direction whose XY components are this vector.</summary>
    public float ToPitch() => MathF.Asin(Math.Clamp(y, -1f, 1f));
}
