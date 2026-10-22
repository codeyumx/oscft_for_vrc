using System;
using System.IO;
using System.Linq;
using System.Reflection;

namespace FaceBridge.Tests;

internal static class TestPaths
{
    private static string Meta(string key) =>
        typeof(TestPaths).Assembly.GetCustomAttributes<AssemblyMetadataAttribute>().Single(a => a.Key == key).Value!;

    public static string RepoRoot => Meta("RepoRoot");

    /// <summary>Build output folder of a solution project (artifacts layout).</summary>
    public static string ProjectOutput(string project) =>
        Path.Combine(Meta("BinRoot"), project, Meta("ConfigurationFolder"));

    public static string ModuleHostExe => Path.Combine(ProjectOutput("FaceBridge.ModuleHost"), "FaceBridge.ModuleHost.exe");

    /// <summary>Directory of the installed .NET runtime this test process runs on (what DOTNET_ROOT must point at).</summary>
    public static string DotnetRoot =>
        Path.GetFullPath(Path.Combine(System.Runtime.InteropServices.RuntimeEnvironment.GetRuntimeDirectory(), "..", "..", ".."));

    public static string NewTempDir(string prefix)
    {
        string dir = Path.Combine(Path.GetTempPath(), "facebridge-tests", prefix + "-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        return dir;
    }

    public static void CopyDirectory(string from, string to)
    {
        Directory.CreateDirectory(to);
        foreach (string file in Directory.GetFiles(from))
        {
            File.Copy(file, Path.Combine(to, Path.GetFileName(file)), overwrite: true);
        }
    }
}
