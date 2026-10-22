using System;
using System.IO;
using System.Net.Sockets;
using Microsoft.Extensions.Logging;

namespace FaceBridge.ModuleHost.Ipc;

/// <summary>Thread-safe sender over the connection to the Rust host. Never throws; a dead connection drops messages.</summary>
internal sealed class IpcChannel
{
    private readonly Stream _stream;
    private readonly ProtocolWriter _writer = new();
    private readonly object _gate = new();
    private bool _broken;

    public IpcChannel(Stream stream)
    {
        _stream = stream;
    }

    public bool IsBroken
    {
        get
        {
            lock (_gate)
            {
                return _broken;
            }
        }
    }

    public bool Send(Message message)
    {
        lock (_gate)
        {
            if (_broken)
            {
                return false;
            }

            try
            {
                _writer.WriteMessage(message);
                _stream.Write(_writer.Written);
                return true;
            }
            catch (Exception e) when (e is IOException or SocketException or ObjectDisposedException)
            {
                _broken = true;
                return false;
            }
            catch (ProtocolException)
            {
                // Over-long message: dropped, the connection itself is fine.
                return false;
            }
        }
    }

    /// <summary>Log line from the module host itself.</summary>
    public void HostLog(LogLevel level, string message, Exception? exception = null)
    {
        Send(new Log(level, 0, "FaceBridge.ModuleHost", message, exception?.ToString() ?? string.Empty));
    }
}
