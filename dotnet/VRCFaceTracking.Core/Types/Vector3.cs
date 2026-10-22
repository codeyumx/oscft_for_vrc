using System;

namespace VRCFaceTracking.Core.Types;

public struct Vector3
{
    public float x;
    public float y;
    public float z;

    public static Vector3 zero => new(0f, 0f, 0f);

    public Vector3(float x, float y, float z)
    {
        this.x = x;
        this.y = y;
        this.z = z;
    }

    public Vector3 FlipXCoordinates() => new(-x, y, z);

    /// <summary>Same projection as <see cref="Vector2.PolarTo2DCartesian"/> applied to (x, y); z is kept.</summary>
    public Vector3 PolarTo2DCartesian(float r)
    {
        return new Vector3(
            r * MathF.Sin(x) * MathF.Cos(y),
            r * MathF.Sin(y),
            z);
    }
}
