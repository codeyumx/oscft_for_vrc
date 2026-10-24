using System;
using Microsoft.Extensions.Logging;

namespace FaceBridge.ModuleHost.Ipc;

internal enum MessageType : byte
{
    // Child -> host
    Hello = 0x01,
    Loaded = 0x02,
    LoadFailed = 0x03,
    InitResult = 0x04,
    Frame = 0x05,
    Log = 0x06,
    Notify = 0x07,
    NotifyClear = 0x08,
    Status = 0x09,
    SettingsSchema = 0x0A,
    SettingsWrite = 0x0B,
    TeardownDone = 0x0C,
    NotifyMessage = 0x0D,

    // Host -> child
    Init = 0x81,
    Settings = 0x82,
    Teardown = 0x83,
    SettingsAction = 0x84,
}

[Flags]
internal enum FrameFlags : byte
{
    None = 0,
    Eye = 1,
    Expression = 2,
    Head = 4,
}

internal abstract record Message
{
    public abstract MessageType Type { get; }

    internal abstract void WritePayload(ProtocolWriter w);
}

internal sealed record Hello(ushort Version, string Token, uint Pid) : Message
{
    public override MessageType Type => MessageType.Hello;

    internal override void WritePayload(ProtocolWriter w)
    {
        w.WriteU16(Version);
        w.WriteString(Token);
        w.WriteU32(Pid);
    }
}

internal sealed record Loaded(string ModuleType, bool SupportsEye, bool SupportsExpression) : Message
{
    public override MessageType Type => MessageType.Loaded;

    internal override void WritePayload(ProtocolWriter w)
    {
        w.WriteString(ModuleType);
        w.WriteBool(SupportsEye);
        w.WriteBool(SupportsExpression);
    }
}

internal sealed record LoadFailed(string Reason) : Message
{
    public override MessageType Type => MessageType.LoadFailed;

    internal override void WritePayload(ProtocolWriter w) => w.WriteString(Reason);
}

internal sealed record InitResult(bool Eye, bool Expression, string Name, byte[][] Images) : Message
{
    public override MessageType Type => MessageType.InitResult;

    public bool Equals(InitResult? other)
    {
        if (other is null || Eye != other.Eye || Expression != other.Expression || Name != other.Name
            || Images.Length != other.Images.Length)
        {
            return false;
        }

        for (int i = 0; i < Images.Length; i++)
        {
            if (!Images[i].AsSpan().SequenceEqual(other.Images[i]))
            {
                return false;
            }
        }

        return true;
    }

    public override int GetHashCode() => HashCode.Combine(Eye, Expression, Name, Images.Length);

    internal override void WritePayload(ProtocolWriter w)
    {
        if (Images.Length > ushort.MaxValue)
        {
            throw new ProtocolException("Too many images.");
        }

        w.WriteBool(Eye);
        w.WriteBool(Expression);
        w.WriteString(Name);
        w.WriteU16((ushort)Images.Length);
        foreach (byte[] image in Images)
        {
            w.WriteBytes(image);
        }
    }
}

/// <summary>
/// One tracking frame. Mutable so the sender can reuse a single instance (and its shape array) per frame.
/// </summary>
internal sealed record Frame : Message
{
    public ulong Seq;
    public FrameFlags Flags;
    public float LeftGazeX, LeftGazeY, LeftOpenness, LeftPupilMm;
    public float RightGazeX, RightGazeY, RightOpenness, RightPupilMm;
    public float MaxDilation, MinDilation, LeftDiameter, RightDiameter;
    public float HeadYaw, HeadPitch, HeadRoll, HeadPosX, HeadPosY, HeadPosZ;

    /// <summary>Shape weights; the first <see cref="ShapeCount"/> entries are valid.</summary>
    public float[] Shapes = Array.Empty<float>();

    public int ShapeCount;

    public override MessageType Type => MessageType.Frame;

    internal override void WritePayload(ProtocolWriter w)
    {
        if ((uint)ShapeCount > ushort.MaxValue || ShapeCount > Shapes.Length)
        {
            throw new ProtocolException("Invalid shape count.");
        }

        w.WriteU64(Seq);
        w.WriteU8((byte)Flags);
        w.WriteF32(LeftGazeX); w.WriteF32(LeftGazeY); w.WriteF32(LeftOpenness); w.WriteF32(LeftPupilMm);
        w.WriteF32(RightGazeX); w.WriteF32(RightGazeY); w.WriteF32(RightOpenness); w.WriteF32(RightPupilMm);
        w.WriteF32(MaxDilation); w.WriteF32(MinDilation); w.WriteF32(LeftDiameter); w.WriteF32(RightDiameter);
        w.WriteF32(HeadYaw); w.WriteF32(HeadPitch); w.WriteF32(HeadRoll);
        w.WriteF32(HeadPosX); w.WriteF32(HeadPosY); w.WriteF32(HeadPosZ);
        w.WriteU16((ushort)ShapeCount);
        w.WriteF32s(Shapes.AsSpan(0, ShapeCount));
    }

    internal static Frame Read(ref ProtocolReader r)
    {
        var f = new Frame
        {
            Seq = r.ReadU64(),
            Flags = (FrameFlags)r.ReadU8(),
            LeftGazeX = r.ReadF32(), LeftGazeY = r.ReadF32(), LeftOpenness = r.ReadF32(), LeftPupilMm = r.ReadF32(),
            RightGazeX = r.ReadF32(), RightGazeY = r.ReadF32(), RightOpenness = r.ReadF32(), RightPupilMm = r.ReadF32(),
            MaxDilation = r.ReadF32(), MinDilation = r.ReadF32(), LeftDiameter = r.ReadF32(), RightDiameter = r.ReadF32(),
            HeadYaw = r.ReadF32(), HeadPitch = r.ReadF32(), HeadRoll = r.ReadF32(),
            HeadPosX = r.ReadF32(), HeadPosY = r.ReadF32(), HeadPosZ = r.ReadF32(),
        };
        int count = r.ReadU16();
        f.Shapes = new float[count];
        r.ReadF32s(f.Shapes);
        f.ShapeCount = count;
        return f;
    }

    public bool Equals(Frame? o)
    {
        return o is not null && Seq == o.Seq && Flags == o.Flags
            && LeftGazeX == o.LeftGazeX && LeftGazeY == o.LeftGazeY && LeftOpenness == o.LeftOpenness && LeftPupilMm == o.LeftPupilMm
            && RightGazeX == o.RightGazeX && RightGazeY == o.RightGazeY && RightOpenness == o.RightOpenness && RightPupilMm == o.RightPupilMm
            && MaxDilation == o.MaxDilation && MinDilation == o.MinDilation && LeftDiameter == o.LeftDiameter && RightDiameter == o.RightDiameter
            && HeadYaw == o.HeadYaw && HeadPitch == o.HeadPitch && HeadRoll == o.HeadRoll
            && HeadPosX == o.HeadPosX && HeadPosY == o.HeadPosY && HeadPosZ == o.HeadPosZ
            && ShapeCount == o.ShapeCount
            && Shapes.AsSpan(0, ShapeCount).SequenceEqual(o.Shapes.AsSpan(0, o.ShapeCount));
    }

    public override int GetHashCode() => HashCode.Combine(Seq, Flags, ShapeCount);
}

internal sealed record Log(LogLevel Level, int EventId, string Category, string Message, string Exception) : Message
{
    public override MessageType Type => MessageType.Log;

    internal override void WritePayload(ProtocolWriter w)
    {
        w.WriteU8((byte)Level);
        w.WriteI32(EventId);
        w.WriteString(Category);
        w.WriteString(Message);
        w.WriteString(Exception);
    }
}

/// <summary>Informational pop-up; never stored.</summary>
internal sealed record Notify(LogLevel Level, string Key, string Message) : Message
{
    public override MessageType Type => MessageType.Notify;

    internal override void WritePayload(ProtocolWriter w)
    {
        w.WriteU8((byte)Level);
        w.WriteString(Key);
        w.WriteString(Message);
    }
}

/// <summary>A message: stays on the Home page until the user dismisses it or the module clears it.</summary>
internal sealed record NotifyMessage(LogLevel Level, string Key, string Message) : Message
{
    public override MessageType Type => MessageType.NotifyMessage;

    internal override void WritePayload(ProtocolWriter w)
    {
        w.WriteU8((byte)Level);
        w.WriteString(Key);
        w.WriteString(Message);
    }
}

internal sealed record NotifyClear(string Key) : Message
{
    public override MessageType Type => MessageType.NotifyClear;

    internal override void WritePayload(ProtocolWriter w) => w.WriteString(Key);
}

internal sealed record Status(sbyte State) : Message
{
    public override MessageType Type => MessageType.Status;

    internal override void WritePayload(ProtocolWriter w) => w.WriteU8(unchecked((byte)State));
}

internal sealed record SettingsSchema(string Json) : Message
{
    public override MessageType Type => MessageType.SettingsSchema;

    internal override void WritePayload(ProtocolWriter w) => w.WriteString(Json);
}

internal sealed record SettingsWrite(string Json) : Message
{
    public override MessageType Type => MessageType.SettingsWrite;

    internal override void WritePayload(ProtocolWriter w) => w.WriteString(Json);
}

internal sealed record TeardownDone : Message
{
    public override MessageType Type => MessageType.TeardownDone;

    internal override void WritePayload(ProtocolWriter w)
    {
    }
}

internal sealed record Init(bool EyeAvailable, bool ExpressionAvailable) : Message
{
    public override MessageType Type => MessageType.Init;

    internal override void WritePayload(ProtocolWriter w)
    {
        w.WriteBool(EyeAvailable);
        w.WriteBool(ExpressionAvailable);
    }
}

internal sealed record Settings(string Json) : Message
{
    public override MessageType Type => MessageType.Settings;

    internal override void WritePayload(ProtocolWriter w) => w.WriteString(Json);
}

internal sealed record Teardown : Message
{
    public override MessageType Type => MessageType.Teardown;

    internal override void WritePayload(ProtocolWriter w)
    {
    }
}

/// <summary>The user pressed the settings button <paramref name="Key"/>.</summary>
internal sealed record SettingsAction(string Key) : Message
{
    public override MessageType Type => MessageType.SettingsAction;

    internal override void WritePayload(ProtocolWriter w) => w.WriteString(Key);
}
