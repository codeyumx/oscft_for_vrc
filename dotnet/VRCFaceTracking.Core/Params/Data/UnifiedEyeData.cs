using System;
using System.Buffers.Binary;
using VRCFaceTracking.Core.Types;

namespace VRCFaceTracking.Core.Params.Data;

public class UnifiedEyeData
{
    /// <summary>Number of bytes <see cref="CopyTo"/> writes.</summary>
    internal const int SerializedBytes = 12 * sizeof(float);

    public UnifiedSingleEyeData Left;
    public UnifiedSingleEyeData Right;

    /// <summary>Largest pupil diameter seen (mm); modules may write it directly.</summary>
    public float _maxDilation;

    /// <summary>Smallest pupil diameter seen (mm); modules may write it directly.</summary>
    public float _minDilation;

    public float _leftDiameter;
    public float _rightDiameter;

    public UnifiedEyeData()
    {
        Left = new UnifiedSingleEyeData();
        Right = new UnifiedSingleEyeData();
    }

    /// <summary>Average of both eyes.</summary>
    public UnifiedSingleEyeData Combined()
    {
        var combined = new UnifiedSingleEyeData
        {
            Gaze = new Vector2((Left.Gaze.x + Right.Gaze.x) * 0.5f, (Left.Gaze.y + Right.Gaze.y) * 0.5f),
            Openness = (Left.Openness + Right.Openness) * 0.5f,
            PupilDiameter_MM = (Left.PupilDiameter_MM + Right.PupilDiameter_MM) * 0.5f,
        };
        return combined;
    }

    public void CopyPropertiesOf(UnifiedEyeData data)
    {
        Left = data.Left;
        Right = data.Right;
        _maxDilation = data._maxDilation;
        _minDilation = data._minDilation;
        _leftDiameter = data._leftDiameter;
        _rightDiameter = data._rightDiameter;
    }

    /// <summary>Writes the eye data as little-endian floats (FaceBridge's own layout), growing the array if needed.</summary>
    public void CopyTo(ref byte[] bytes)
    {
        if (bytes == null || bytes.Length < SerializedBytes)
        {
            bytes = new byte[SerializedBytes];
        }

        WriteTo(bytes.AsSpan());
    }

    internal void WriteTo(Span<byte> span)
    {
        int o = 0;
        Write(span, ref o, Left.Gaze.x);
        Write(span, ref o, Left.Gaze.y);
        Write(span, ref o, Left.Openness);
        Write(span, ref o, Left.PupilDiameter_MM);
        Write(span, ref o, Right.Gaze.x);
        Write(span, ref o, Right.Gaze.y);
        Write(span, ref o, Right.Openness);
        Write(span, ref o, Right.PupilDiameter_MM);
        Write(span, ref o, _maxDilation);
        Write(span, ref o, _minDilation);
        Write(span, ref o, _leftDiameter);
        Write(span, ref o, _rightDiameter);
    }

    internal static void Write(Span<byte> span, ref int offset, float value)
    {
        BinaryPrimitives.WriteSingleLittleEndian(span.Slice(offset, sizeof(float)), value);
        offset += sizeof(float);
    }
}
