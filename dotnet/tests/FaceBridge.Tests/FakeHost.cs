using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using FaceBridge.ModuleHost.Ipc;

namespace FaceBridge.Tests;

/// <summary>The Rust host's side of the protocol, in-process: listens, launches the module host, reads/sends messages.</summary>
internal sealed class FakeHost : IDisposable
{
    public const string Token = "0123456789abcdef0123456789abcdef";
    private static readonly TimeSpan DefaultTimeout = TimeSpan.FromSeconds(30);

    private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
    private readonly BlockingCollection<Message> _inbox = new();
    private readonly StringBuilder _stdio = new();
    private readonly List<Message> _pending = new();
    private TcpClient? _client;
    private IpcChannel? _channel;

    public FakeHost()
    {
        _listener.Start();
    }

    public Process? Process { get; private set; }

    public int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;

    /// <summary>Every message received so far, in order.</summary>
    public List<Message> All { get; } = new();

    public string ProcessOutput
    {
        get
        {
            lock (_stdio)
            {
                return _stdio.ToString();
            }
        }
    }

    public void Start(string modulePath, string dataDir, int? parentPid = null, IDictionary<string, string>? env = null, int? port = null)
    {
        var psi = new ProcessStartInfo(TestPaths.ModuleHostExe)
        {
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true,
        };
        psi.Environment["DOTNET_ROOT"] = TestPaths.DotnetRoot;
        psi.Environment["DOTNET_ROOT(x86)"] = TestPaths.DotnetRoot;
        if (env != null)
        {
            foreach ((string k, string v) in env)
            {
                psi.Environment[k] = v;
            }
        }

        foreach (string a in new[]
        {
            "--connect", $"127.0.0.1:{port ?? Port}", "--token", Token, "--module", modulePath,
            "--data-dir", dataDir, "--parent-pid", (parentPid ?? Environment.ProcessId).ToString(),
        })
        {
            psi.ArgumentList.Add(a);
        }

        Process = Process.Start(psi)!;
        Process.OutputDataReceived += (_, e) => Capture("out", e.Data);
        Process.ErrorDataReceived += (_, e) => Capture("err", e.Data);
        Process.BeginOutputReadLine();
        Process.BeginErrorReadLine();
    }

    /// <summary>Accepts the child's connection and starts the reader thread.</summary>
    public void Accept()
    {
        Task<TcpClient> accept = _listener.AcceptTcpClientAsync();
        if (!accept.Wait(DefaultTimeout))
        {
            throw new TimeoutException("The module host never connected. " + ProcessOutput);
        }

        _client = accept.Result;
        _client.NoDelay = true;
        NetworkStream stream = _client.GetStream();
        _channel = new IpcChannel(stream);
        new Thread(() => ReadLoop(stream)) { IsBackground = true, Name = "fake-host-reader" }.Start();
    }

    public void Send(Message message)
    {
        Assert(_channel!.Send(message), "send failed");
    }

    /// <summary>
    /// Takes the first not-yet-consumed message of type <typeparamref name="T"/> that satisfies <paramref name="predicate"/>
    /// (waiting for more to arrive if needed). Other messages stay available for later calls, so the order of
    /// expectations does not have to match the order of arrival.
    /// </summary>
    public T Expect<T>(Func<T, bool>? predicate = null, TimeSpan? timeout = null)
        where T : Message
    {
        DateTime deadline = DateTime.UtcNow + (timeout ?? DefaultTimeout);
        while (true)
        {
            int index = _pending.FindIndex(m => m is T typed && (predicate == null || predicate(typed)));
            if (index >= 0)
            {
                var found = (T)_pending[index];
                _pending.RemoveAt(index);
                return found;
            }

            TimeSpan left = deadline - DateTime.UtcNow;
            if (left <= TimeSpan.Zero || !_inbox.TryTake(out Message? message, (int)left.TotalMilliseconds))
            {
                throw new TimeoutException($"No matching {typeof(T).Name} within the timeout. Received: {string.Join(" | ", All.Where(m => m is not Frame))}\nProcess output: {ProcessOutput}");
            }

            All.Add(message);
            _pending.Add(message);
        }
    }

    /// <summary>Closes the TCP connection from the host side.</summary>
    public void CloseConnection() => _client!.Dispose();

    public int WaitForExit(TimeSpan? timeout = null)
    {
        if (!Process!.WaitForExit((int)(timeout ?? DefaultTimeout).TotalMilliseconds))
        {
            throw new TimeoutException("Module host did not exit. Output: " + ProcessOutput);
        }

        Process.WaitForExit(); // flush async output
        return Process.ExitCode;
    }

    public void Dispose()
    {
        try
        {
            if (Process is { HasExited: false })
            {
                Process.Kill(entireProcessTree: true);
            }
        }
        catch (InvalidOperationException)
        {
        }

        Process?.Dispose();
        _client?.Dispose();
        _listener.Stop();
    }

    private void ReadLoop(Stream stream)
    {
        byte[] scratch = new byte[4096];
        try
        {
            while (Protocol.ReadMessage(stream, ref scratch) is { } message)
            {
                _inbox.Add(message);
            }
        }
        catch (Exception e) when (e is IOException or ObjectDisposedException or ProtocolException)
        {
        }
    }

    private void Capture(string stream, string? line)
    {
        if (line != null)
        {
            lock (_stdio)
            {
                _stdio.Append('[').Append(stream).Append("] ").AppendLine(line);
            }
        }
    }

    private static void Assert(bool condition, string message)
    {
        if (!condition)
        {
            throw new InvalidOperationException(message);
        }
    }
}
