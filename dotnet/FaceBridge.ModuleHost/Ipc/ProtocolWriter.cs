using System;
using System.Buffers.Binary;
using System.Text;

namespace FaceBridge.ModuleHost.Ipc;

/// <summary>Growable little-endian writer; reused across messages to avoid per-frame allocation.</summary>
internal sealed class ProtocolWriter
{
    private static readonly UTF8Encoding Utf8 = new(false); // lone surrogates in module text become U+FFFD
    private byte[] _buffer = new byte[1024];
    private int _length;

    public ReadOnlySpan<byte> Written => _buffer.AsSpan(0, _length);

    public void Reset() => _length = 0;

    public void WriteU8(byte value)
    {
        Reserve(1)[0] = value;
    }

    public void WriteBool(bool value) => WriteU8(value ? (byte)1 : (byte)0);

    public void WriteU16(ushort value) => BinaryPrimitives.WriteUInt16LittleEndian(Reserve(2), value);

    public void WriteU32(uint value) => BinaryPrimitives.WriteUInt32LittleEndian(Reserve(4), value);

    public void WriteI32(int value) => BinaryPrimitives.WriteInt32LittleEndian(Reserve(4), value);

    public void WriteU64(ulong value) => BinaryPrimitives.WriteUInt64LittleEndian(Reserve(8), value);

    public void WriteF32(float value) => BinaryPrimitives.WriteSingleLittleEndian(Reserve(4), value);

    public void WriteF32s(ReadOnlySpan<float> values)
    {
        Span<byte> dst = Reserve(values.Length * 4);
        for (int i = 0; i < values.Length; i++)
        {
            BinaryPrimitives.WriteSingleLittleEndian(dst.Slice(i * 4, 4), values[i]);
        }
    }

    public void WriteBytes(ReadOnlySpan<byte> value)
    {
        WriteU32((uint)value.Length);
        value.CopyTo(Reserve(value.Length));
    }

    public void WriteString(string value)
    {
        int byteCount = Utf8.GetByteCount(value);
        WriteU32((uint)byteCount);
        Utf8.GetBytes(value, Reserve(byteCount));
    }

    /// <summary>Writes the complete frame: length prefix, type byte, payload.</summary>
    public void WriteMessage(Message message)
    {
        Reset();
        Reserve(4);
        WriteU8((byte)message.Type);
        message.WritePayload(this);
        int bodyLength = _length - 4;
        if (bodyLength > Protocol.MaxMessageLength)
        {
            throw new ProtocolException($"Message of {bodyLength} bytes exceeds the {Protocol.MaxMessageLength} byte limit.");
        }

        BinaryPrimitives.WriteUInt32LittleEndian(_buffer.AsSpan(0, 4), (uint)bodyLength);
    }

    private Span<byte> Reserve(int count)
    {
        int needed = _length + count;
        if (needed > _buffer.Length)
        {
            Array.Resize(ref _buffer, Math.Max(needed, _buffer.Length * 2));
        }

        Span<byte> span = _buffer.AsSpan(_length, count);
        _length = needed;
        return span;
    }
}
