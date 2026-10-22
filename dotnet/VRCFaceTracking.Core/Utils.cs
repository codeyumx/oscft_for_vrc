using System;
using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Security.Cryptography;

namespace VRCFaceTracking.Core;

/// <summary>Helper statics some modules use (Win32 interop, free ports, data directories).</summary>
public static class Utils
{
    public const int PROCESS_VM_READ = 16;

    public static readonly bool HasAdmin = DataPaths.HasAdmin;

    public static readonly string UserAccessibleDataDirectory = DataPaths.UserAccessibleDataDirectory;

    public static readonly string PersistentDataDirectory = DataPaths.PersistentDataDirectory;

    public static readonly string CustomLibsDirectory = DataPaths.CustomLibsDirectory;

    [DllImport("winmm.dll", EntryPoint = "timeBeginPeriod")]
    public static extern uint TimeBeginPeriod(uint uMilliseconds);

    [DllImport("winmm.dll", EntryPoint = "timeEndPeriod")]
    public static extern uint TimeEndPeriod(uint uMilliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr OpenProcess(int dwDesiredAccess, bool bInheritHandle, int dwProcessId);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool ReadProcessMemory(int hProcess, IntPtr lpBaseAddress, byte[] lpBuffer, int dwSize, ref int lpNumberOfBytesRead);

    [DllImport("kernel32.dll", EntryPoint = "DeleteFileW", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool DeleteFile(string lpFileName);

    [DllImport("kernel32.dll", EntryPoint = "GetFileAttributesW", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern uint GetFileAttributes(string lpFileName);

    public static int GetRandomFreePort()
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        try
        {
            return ((IPEndPoint)listener.LocalEndpoint).Port;
        }
        finally
        {
            listener.Stop();
        }
    }

    public static string GetRandomChars(int num)
    {
        const string alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        if (num <= 0)
        {
            return string.Empty;
        }

        return string.Create(num, alphabet, static (span, chars) =>
        {
            for (int i = 0; i < span.Length; i++)
            {
                span[i] = chars[RandomNumberGenerator.GetInt32(chars.Length)];
            }
        });
    }

    public static void KillAllProcessesOfName(string name)
    {
        foreach (Process process in Process.GetProcessesByName(name))
        {
            try
            {
                process.Kill();
            }
            catch (Exception)
            {
                // Already exited or access denied; nothing else to do.
            }
            finally
            {
                process.Dispose();
            }
        }
    }
}
