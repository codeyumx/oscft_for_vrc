using System;
using VRCFaceTracking.Core.Params.Expressions;

namespace VRCFaceTracking.Core.Params.Data;

public class UnifiedTrackingData
{
    /// <summary>Constant kept for source/binary compatibility; FaceBridge uses its own serialization.</summary>
    public const int SerializedLength = 102;

    private const int HeadBytes = 6 * sizeof(float);

    public UnifiedEyeData Eye;
    public UnifiedExpressionShape[] Shapes;
    public UnifiedHeadData Head;

    public UnifiedTrackingData()
    {
        Eye = new UnifiedEyeData();
        Shapes = new UnifiedExpressionShape[(int)UnifiedExpressions.Max];
    }

    public void CopyPropertiesOf(UnifiedTrackingData data)
    {
        Eye.CopyPropertiesOf(data.Eye);
        Head = data.Head;
        int count = Math.Min(Shapes.Length, data.Shapes.Length);
        Array.Copy(data.Shapes, Shapes, count);
    }

    /// <summary>Writes eye, head and shapes as little-endian floats (FaceBridge's own layout), growing the array if needed.</summary>
    public void CopyTo(ref byte[] bytes)
    {
        int needed = UnifiedEyeData.SerializedBytes + HeadBytes + Shapes.Length * sizeof(float);
        if (bytes == null || bytes.Length < needed)
        {
            bytes = new byte[needed];
        }

        Span<byte> span = bytes.AsSpan();
        Eye.WriteTo(span);
        int o = UnifiedEyeData.SerializedBytes;
        UnifiedEyeData.Write(span, ref o, Head.HeadYaw);
        UnifiedEyeData.Write(span, ref o, Head.HeadPitch);
        UnifiedEyeData.Write(span, ref o, Head.HeadRoll);
        UnifiedEyeData.Write(span, ref o, Head.HeadPosX);
        UnifiedEyeData.Write(span, ref o, Head.HeadPosY);
        UnifiedEyeData.Write(span, ref o, Head.HeadPosZ);
        for (int i = 0; i < Shapes.Length; i++)
        {
            UnifiedEyeData.Write(span, ref o, Shapes[i].Weight);
        }
    }
}
