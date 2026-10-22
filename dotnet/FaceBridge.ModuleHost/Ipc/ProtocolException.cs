using System;

namespace FaceBridge.ModuleHost.Ipc;

/// <summary>The peer sent bytes that are not valid protocol (unknown type, bad length, truncated or trailing data).</summary>
internal sealed class ProtocolException : Exception
{
    public ProtocolException(string message)
        : base(message)
    {
    }
}
