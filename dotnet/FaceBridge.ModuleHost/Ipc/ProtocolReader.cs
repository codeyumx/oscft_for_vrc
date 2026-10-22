using System;
using System.Buffers.Binary;
using System.Text;

namespace FaceBridge.ModuleHost.Ipc;

/// <summary>Bounds-checked little-endian reader over one message payload.</summary>
internal ref struct ProtocolReader
{
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private ReadOnlySpan<byte> _data;

    public ProtocolReader(ReadOnlySpan<byte> data)
    {
        _data = data;
    }

    public readonly int Remaining => _data.Length;

    public byte ReadU8() => Take(1)[0];

    public bool ReadBool()
    {
        byte b = ReadU8();
        if (b > 1)
        {
            throw new ProtocolException($"Invalid bool value {b}.");
        }

        return b == 1;
    }

    public ushort ReadU16() => BinaryPrimitives.ReadUInt16LittleEndian(Take(2));

    public uint ReadU32() => BinaryPrimitives.ReadUInt32LittleEndian(Take(4));

    public int ReadI32() => BinaryPrimitives.ReadInt32LittleEndian(Take(4));

    public ulong ReadU64() => BinaryPrimitives.ReadUInt64LittleEndian(Take(8));

    public float ReadF32() => BinaryPrimitives.ReadSingleLittleEndian(Take(4));

    public void ReadF32s(Span<float> destination)
    {
        ReadOnlySpan<byte> src = Take(destination.Length * 4);
        for (int i = 0; i < destination.Length; i++)
        {
            destination[i] = BinaryPrimitives.ReadSingleLittleEndian(src.Slice(i * 4, 4));
        }
    }

    public byte[] ReadBytes()
    {
        uint length = ReadU32();
        if (length > Remaining)
        {
            throw new ProtocolException("Byte array extends past the end of the message.");
        }

        return Take((int)length).ToArray();
    }

    public string ReadString()
    {
        uint length = ReadU32();
        if (length > Remaining)
        {
            throw new ProtocolException("String extends past the end of the message.");
        }

        try
        {
            return Utf8.GetString(Take((int)length));
        }
        catch (ArgumentException)
        {
            throw new ProtocolException("String is not valid UTF-8.");
        }
    }

    private ReadOnlySpan<byte> Take(int count)
    {
        if (count < 0 || count > _data.Length)
        {
            throw new ProtocolException("Message is truncated.");
        }

        ReadOnlySpan<byte> head = _data.Slice(0, count);
        _data = _data.Slice(count);
        return head;
    }
}
