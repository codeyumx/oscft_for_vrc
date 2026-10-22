using System;
using System.IO;
using System.Text;
using FaceBridge.ModuleHost.Ipc;
using Microsoft.Extensions.Logging;

namespace FaceBridge.ModuleHost.Logging;

/// <summary>Turns everything the module writes to stdout/stderr into IPC <c>Log</c> messages, one per line.</summary>
internal static class ConsoleRedirect
{
    public static void Install(IpcChannel channel)
    {
        Console.SetOut(TextWriter.Synchronized(new LineLogWriter(channel, LogLevel.Information, "Console.Out")));
        Console.SetError(TextWriter.Synchronized(new LineLogWriter(channel, LogLevel.Error, "Console.Error")));
    }

    internal sealed class LineLogWriter : TextWriter
    {
        private readonly IpcChannel _channel;
        private readonly LogLevel _level;
        private readonly string _category;
        private readonly StringBuilder _line = new();

        public LineLogWriter(IpcChannel channel, LogLevel level, string category)
        {
            _channel = channel;
            _level = level;
            _category = category;
        }

        public override Encoding Encoding => Encoding.UTF8;

        public override void Write(char value)
        {
            if (value == '\n')
            {
                EmitLine();
            }
            else
            {
                _line.Append(value);
            }
        }

        public override void Write(string? value)
        {
            if (value != null)
            {
                Write(value.AsSpan());
            }
        }

        public override void Write(char[] buffer, int index, int count) => Write(buffer.AsSpan(index, count));

        public override void Write(ReadOnlySpan<char> buffer)
        {
            while (!buffer.IsEmpty)
            {
                int newline = buffer.IndexOf('\n');
                if (newline < 0)
                {
                    _line.Append(buffer);
                    return;
                }

                _line.Append(buffer.Slice(0, newline));
                EmitLine();
                buffer = buffer.Slice(newline + 1);
            }
        }

        public override void WriteLine(string? value)
        {
            Write(value);
            EmitLine();
        }

        public override void Flush()
        {
            if (_line.Length > 0)
            {
                EmitLine();
            }
        }

        private void EmitLine()
        {
            string text = _line.ToString().TrimEnd('\r');
            _line.Clear();
            if (text.Length > 0)
            {
                _channel.Send(new Log(_level, 0, _category, text, string.Empty));
            }
        }
    }
}
