using System;
using System.IO;
using System.Security.Principal;

namespace VRCFaceTracking;

/// <summary>
/// Shared implementation of the directory/admin values both <c>Utils</c> classes expose. The FaceBridge module host
/// sets the FACEBRIDGE_* environment variables before the module is loaded; without them the paths fall back to
/// per-user FaceBridge folders.
/// </summary>
internal static class DataPaths
{
    internal const string UserDirVariable = "FACEBRIDGE_USER_DATA_DIR";
    internal const string ModuleDirVariable = "FACEBRIDGE_MODULE_DATA_DIR";
    internal const string ModulesDirVariable = "FACEBRIDGE_MODULES_DIR";

    internal static readonly string UserAccessibleDataDirectory =
        FromEnvironment(UserDirVariable) ?? Path.Combine(AppData(), "FaceBridge");

    internal static readonly string PersistentDataDirectory =
        FromEnvironment(ModuleDirVariable) ?? Path.Combine(UserAccessibleDataDirectory, "module-data");

    internal static readonly string CustomLibsDirectory =
        FromEnvironment(ModulesDirVariable) ?? Path.Combine(UserAccessibleDataDirectory, "modules");

    internal static readonly bool HasAdmin = DetectAdmin();

    private static string FromEnvironment(string name)
    {
        string value = Environment.GetEnvironmentVariable(name);
        return string.IsNullOrWhiteSpace(value) ? null : value;
    }

    private static string AppData() => Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);

    private static bool DetectAdmin()
    {
        if (!OperatingSystem.IsWindows())
        {
            return false;
        }

        try
        {
            using var identity = WindowsIdentity.GetCurrent();
            return new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator);
        }
        catch (Exception)
        {
            return false;
        }
    }
}
