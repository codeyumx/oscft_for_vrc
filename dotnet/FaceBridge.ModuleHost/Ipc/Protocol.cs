using System;
using System.IO;
using Microsoft.Extensions.Logging;

namespace FaceBridge.ModuleHost.Ipc;

internal static class Protocol
{
    public const ushort Version = 1;

    /// <summary>Largest allowed value of the length prefix (type byte + payload).</summary>
    public const int MaxMessageLength = 16 * 1024 * 1024;

    public static byte[] Encode(Message message)
    {
        var writer = new ProtocolWriter();
        writer.WriteMessage(message);
        return writer.Written.ToArray();
    }

    /// <summary>Decodes one message body (type byte followed by the payload; without the length prefix).</summary>
    public static Message Decode(ReadOnlySpan<byte> body)
    {
        if (body.Length == 0)
        {
            throw new ProtocolException("Empty message.");
        }

        var r = new ProtocolReader(body.Slice(1));
        Message message = (MessageType)body[0] switch
        {
            MessageType.Hello => new Hello(r.ReadU16(), r.ReadString(), r.ReadU32()),
            MessageType.Loaded => new Loaded(r.ReadString(), r.ReadBool(), r.ReadBool()),
            MessageType.LoadFailed => new LoadFailed(r.ReadString()),
            MessageType.InitResult => ReadInitResult(ref r),
            MessageType.Frame => Frame.Read(ref r),
            MessageType.Log => new Log((LogLevel)r.ReadU8(), r.ReadI32(), r.ReadString(), r.ReadString(), r.ReadString()),
            MessageType.Notify => new Notify((LogLevel)r.ReadU8(), r.ReadString(), r.ReadString()),
            MessageType.NotifyClear => new NotifyClear(r.ReadString()),
            MessageType.Status => new Status(unchecked((sbyte)r.ReadU8())),
            MessageType.SettingsSchema => new SettingsSchema(r.ReadString()),
            MessageType.SettingsWrite => new SettingsWrite(r.ReadString()),
            MessageType.TeardownDone => new TeardownDone(),
            MessageType.Init => new Init(r.ReadBool(), r.ReadBool()),
            MessageType.Settings => new Settings(r.ReadString()),
            MessageType.Teardown => new Teardown(),
            MessageType.SettingsAction => new SettingsAction(r.ReadString()),
            _ => throw new ProtocolException($"Unknown message type 0x{body[0]:x2}."),
        };
        if (r.Remaining != 0)
        {
            throw new ProtocolException($"{r.Remaining} unexpected trailing bytes in message 0x{body[0]:x2}.");
        }

        return message;
    }

    private static InitResult ReadInitResult(ref ProtocolReader r)
    {
        bool eye = r.ReadBool();
        bool expression = r.ReadBool();
        string name = r.ReadString();
        int count = r.ReadU16();
        var images = new byte[count][];
        for (int i = 0; i < count; i++)
        {
            images[i] = r.ReadBytes();
        }

        return new InitResult(eye, expression, name, images);
    }

    /// <summary>Reads one framed message; returns null on a clean end of stream at a message boundary.</summary>
    public static Message? ReadMessage(Stream stream, ref byte[] scratch)
    {
        Span<byte> prefix = stackalloc byte[4];
        int got = ReadFully(stream, prefix);
        if (got == 0)
        {
            return null;
        }

        if (got < 4)
        {
            throw new EndOfStreamException("Connection closed inside a length prefix.");
        }

        uint length = System.Buffers.Binary.BinaryPrimitives.ReadUInt32LittleEndian(prefix);
        if (length == 0 || length > MaxMessageLength)
        {
            throw new ProtocolException($"Invalid message length {length}.");
        }

        if (scratch.Length < length)
        {
            scratch = new byte[Math.Max((int)length, scratch.Length * 2)];
        }

        Span<byte> body = scratch.AsSpan(0, (int)length);
        if (ReadFully(stream, body) < body.Length)
        {
            throw new EndOfStreamException("Connection closed inside a message.");
        }

        return Decode(body);
    }

    private static int ReadFully(Stream stream, Span<byte> buffer)
    {
        int total = 0;
        while (total < buffer.Length)
        {
            int n = stream.Read(buffer.Slice(total));
            if (n == 0)
            {
                break;
            }

            total += n;
        }

        return total;
    }
}
