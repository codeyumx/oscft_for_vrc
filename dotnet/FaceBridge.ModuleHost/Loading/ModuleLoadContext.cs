using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Runtime.Loader;

namespace FaceBridge.ModuleHost.Loading;

/// <summary>
/// Load context for one module. The compat assemblies, the logging abstractions and FaceBridge.Sdk always come from
/// the host (whatever version the module asks for) so module and host share one set of types. Other managed
/// dependencies come from the module's folder when that copy is newer than the host's, else from the host/framework.
/// Native libraries are looked up in the module's folder.
/// </summary>
internal sealed class ModuleLoadContext : AssemblyLoadContext
{
    private static readonly HashSet<string> HostMapped = new(StringComparer.OrdinalIgnoreCase)
    {
        "VRCFaceTracking.Core",
        "VRCFaceTracking.SDK",
        "Microsoft.Extensions.Logging.Abstractions",
        "FaceBridge.Sdk",
    };

    private readonly string _moduleDirectory;

    public ModuleLoadContext(string moduleDirectory)
        : base("module:" + moduleDirectory, isCollectible: false)
    {
        _moduleDirectory = moduleDirectory;
    }

    public static bool IsHostMapped(string simpleName) => HostMapped.Contains(simpleName);

    protected override Assembly? Load(AssemblyName assemblyName)
    {
        string? name = assemblyName.Name;
        if (name == null)
        {
            return null;
        }

        if (HostMapped.Contains(name))
        {
            return Default.LoadFromAssemblyName(new AssemblyName(name));
        }

        string candidate = Path.Combine(_moduleDirectory, name + ".dll");
        if (!File.Exists(candidate))
        {
            return null; // shared framework / host
        }

        Version? moduleVersion;
        try
        {
            moduleVersion = AssemblyName.GetAssemblyName(candidate).Version;
        }
        catch (BadImageFormatException)
        {
            return null;
        }

        Assembly? hostCopy = TryLoadFromHost(name);
        if (hostCopy != null && moduleVersion != null && hostCopy.GetName().Version >= moduleVersion)
        {
            return hostCopy;
        }

        return LoadFromAssemblyPath(candidate);
    }

    protected override IntPtr LoadUnmanagedDll(string unmanagedDllName)
    {
        foreach (string path in NativeCandidates(unmanagedDllName))
        {
            if (File.Exists(path) && NativeLibrary.TryLoad(path, out IntPtr handle))
            {
                return handle;
            }
        }

        return IntPtr.Zero;
    }

    private IEnumerable<string> NativeCandidates(string name)
    {
        string[] names = Path.HasExtension(name) ? new[] { name, name + ".dll" } : new[] { name + ".dll", name };
        string[] folders =
        {
            _moduleDirectory,
            Path.Combine(_moduleDirectory, "runtimes", "win-x64", "native"),
            Path.Combine(_moduleDirectory, "runtimes", "win", "native"),
        };

        foreach (string folder in folders)
        {
            foreach (string n in names)
            {
                yield return Path.Combine(folder, n);
            }
        }
    }

    private static Assembly? TryLoadFromHost(string simpleName)
    {
        try
        {
            return Default.LoadFromAssemblyName(new AssemblyName(simpleName));
        }
        catch (FileNotFoundException)
        {
            return null;
        }
    }
}
