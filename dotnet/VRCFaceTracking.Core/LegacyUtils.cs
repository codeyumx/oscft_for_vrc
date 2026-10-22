using System;
using System.Runtime.InteropServices;

namespace VRCFaceTracking;

/// <summary>Older location of <see cref="VRCFaceTracking.Core.Utils"/>' Win32 helpers; some modules still bind to it.</summary>
public class Utils
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

    public Utils()
    {
    }
}
