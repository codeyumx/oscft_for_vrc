using System;
using System.IO;
using System.Linq;
using FaceBridge.ModuleHost.Ipc;
using Microsoft.Extensions.Logging;
using Xunit;

namespace FaceBridge.Tests;

public class ProtocolTests
{
    private static byte[] Hex(string hex) => Convert.FromHexString(hex.Replace(" ", string.Empty));

    private static Frame VectorFrame() => new()
    {
        Seq = 7,
        Flags = FrameFlags.Eye | FrameFlags.Expression,
        LeftGazeX = 0.25f, LeftGazeY = -0.5f, LeftOpenness = 1.0f, LeftPupilMm = 3.5f,
        RightGazeX = 0, RightGazeY = 0, RightOpenness = 0.75f, RightPupilMm = 4.0f,
        MaxDilation = 6, MinDilation = 2, LeftDiameter = 3.5f, RightDiameter = 4,
        Shapes = new[] { 0f, 0.5f, 1f },
        ShapeCount = 3,
    };

    private static (string Hex, Message Message)[] VectorList => new (string, Message)[]
    {
        (
            "2b 00 00 00 01 01 00 20 00 00 00 30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66 30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66 92 10 00 00",
            new Hello(1, "0123456789abcdef0123456789abcdef", 4242)
        ),
        ("03 00 00 00 81 01 00", new Init(true, false)),
        (
            "27 00 00 00 07 03 0a 00 00 00 6e 6f 2d 68 65 61 64 73 65 74 13 00 00 00 4e 6f 20 68 65 61 64 73 65 74 20 61 6e 73 77 65 72 65 64",
            new Notify(LogLevel.Warning, "no-headset", "No headset answered")
        ),
        (
            "60 00 00 00 05 07 00 00 00 00 00 00 00 03 00 00 80 3e 00 00 00 bf 00 00 80 3f 00 00 60 40 00 00 00 00 00 00 00 00 00 00 40 3f 00 00 80 40 00 00 c0 40 00 00 00 40 00 00 60 40 00 00 80 40 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 03 00 00 00 00 00 00 00 00 3f 00 00 80 3f",
            VectorFrame()
        ),
        ("09 00 00 00 84 04 00 00 00 70 61 69 72", new SettingsAction("pair")),
    };

    public static TheoryData<int> Vectors => new(Enumerable.Range(0, VectorList.Length));

    [Theory]
    [MemberData(nameof(Vectors))]
    public void Encodes_spec_test_vectors_exactly(int index)
    {
        (string hex, Message message) = VectorList[index];
        Assert.Equal(Hex(hex), Protocol.Encode(message));
    }

    [Theory]
    [MemberData(nameof(Vectors))]
    public void Decodes_spec_test_vectors(int index)
    {
        (string hex, Message expected) = VectorList[index];
        byte[] bytes = Hex(hex);
        Assert.Equal(bytes.Length - 4, BitConverter.ToInt32(bytes, 0));
        Assert.Equal(expected, Protocol.Decode(bytes.AsSpan(4)));
    }

    private static Message[] MessageList => new Message[]
    {
        new Hello(1, "tok", 1),
        new Loaded("My.Module", true, false),
        new LoadFailed("nope é中"),
        new InitResult(true, true, "Name", new[] { new byte[] { 1, 2, 3 }, Array.Empty<byte>() }),
        new InitResult(false, false, string.Empty, Array.Empty<byte[]>()),
        VectorFrame(),
        new Log(LogLevel.Error, 7400, "Cat", "message", "System.Exception: x"),
        new Notify(LogLevel.Information, string.Empty, "hi"),
        new NotifyClear("k"),
        new Status(-1),
        new Status(1),
        new SettingsSchema("{\"fields\":[]}"),
        new SettingsWrite("{\"a\":1}"),
        new TeardownDone(),
        new Init(false, true),
        new Settings("{}"),
        new Teardown(),
        new SettingsAction(string.Empty),
        new SettingsAction("pair é中"),
    };

    public static TheoryData<int> AllMessages => new(Enumerable.Range(0, MessageList.Length));

    [Theory]
    [MemberData(nameof(AllMessages))]
    public void Every_message_round_trips(int index)
    {
        Message message = MessageList[index];
        byte[] bytes = Protocol.Encode(message);
        Assert.Equal(bytes.Length - 4, (int)BitConverter.ToUInt32(bytes, 0));
        Assert.Equal(message, Protocol.Decode(bytes.AsSpan(4)));
    }

    [Fact]
    public void Reads_a_stream_of_messages_delivered_one_byte_at_a_time()
    {
        var ms = new MemoryStream();
        foreach (Message m in new Message[] { new Init(true, true), new Notify(LogLevel.Debug, "a", "b"), new Teardown() })
        {
            ms.Write(Protocol.Encode(m));
        }

        ms.Position = 0;
        var stream = new OneByteStream(ms);
        byte[] scratch = new byte[1];
        Assert.Equal(new Init(true, true), Protocol.ReadMessage(stream, ref scratch));
        Assert.Equal(new Notify(LogLevel.Debug, "a", "b"), Protocol.ReadMessage(stream, ref scratch));
        Assert.Equal(new Teardown(), Protocol.ReadMessage(stream, ref scratch));
        Assert.Null(Protocol.ReadMessage(stream, ref scratch));
    }

    [Fact]
    public void Rejects_unknown_type_and_trailing_bytes_and_truncation()
    {
        Assert.Throws<ProtocolException>(() => Protocol.Decode(new byte[] { 0x55 }));
        Assert.Throws<ProtocolException>(() => Protocol.Decode(new byte[] { 0x83, 0x00 })); // Teardown with payload
        Assert.Throws<ProtocolException>(() => Protocol.Decode(new byte[] { 0x81, 0x01 })); // Init truncated
        Assert.Throws<ProtocolException>(() => Protocol.Decode(new byte[] { 0x81, 0x02, 0x00 })); // bool 2
        Assert.Throws<ProtocolException>(() => Protocol.Decode(new byte[] { 0x82, 0xff, 0xff, 0xff, 0x7f })); // string longer than message
        Assert.Throws<ProtocolException>(() => Protocol.Decode(new byte[] { 0x82, 0x01, 0x00, 0x00, 0x00, 0xff })); // invalid UTF-8
        Assert.Throws<ProtocolException>(() => Protocol.Decode(ReadOnlySpan<byte>.Empty));
    }

    [Fact]
    public void Rejects_bad_length_prefixes_and_mid_message_eof()
    {
        byte[] scratch = new byte[16];
        Assert.Throws<ProtocolException>(() => Protocol.ReadMessage(new MemoryStream(new byte[] { 0, 0, 0, 0 }), ref scratch));
        byte[] huge = BitConverter.GetBytes((uint)Protocol.MaxMessageLength + 1);
        Assert.Throws<ProtocolException>(() =>
        {
            byte[] s = new byte[16];
            return Protocol.ReadMessage(new MemoryStream(huge), ref s);
        });
        Assert.Throws<EndOfStreamException>(() =>
        {
            byte[] s = new byte[16];
            return Protocol.ReadMessage(new MemoryStream(new byte[] { 5, 0, 0, 0, 0x82 }), ref s);
        });
    }

    [Fact]
    public void Refuses_to_encode_a_message_over_the_size_limit()
    {
        var huge = new Settings(new string('x', Protocol.MaxMessageLength));
        Assert.Throws<ProtocolException>(() => Protocol.Encode(huge));
    }

    [Fact]
    public void Frame_with_zero_shapes_and_reused_buffer_encodes_only_the_valid_prefix()
    {
        Frame f = VectorFrame();
        f.Shapes = new float[88];
        f.ShapeCount = 1;
        Assert.Equal(f, Protocol.Decode(Protocol.Encode(f).AsSpan(4)));
        Assert.Equal(1, ((Frame)Protocol.Decode(Protocol.Encode(f).AsSpan(4))).ShapeCount);
    }

    private sealed class OneByteStream : Stream
    {
        private readonly Stream _inner;

        public OneByteStream(Stream inner)
        {
            _inner = inner;
        }

        public override int Read(byte[] buffer, int offset, int count) => _inner.Read(buffer, offset, Math.Min(1, count));

        public override int Read(Span<byte> buffer) => _inner.Read(buffer.Slice(0, Math.Min(1, buffer.Length)));

        public override bool CanRead => true;
        public override bool CanSeek => false;
        public override bool CanWrite => false;
        public override long Length => throw new NotSupportedException();
        public override long Position { get => throw new NotSupportedException(); set => throw new NotSupportedException(); }
        public override void Flush() { }
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
    }
}
