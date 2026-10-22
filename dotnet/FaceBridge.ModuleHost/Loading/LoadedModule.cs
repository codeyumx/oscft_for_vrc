using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Extensions.Logging;
using VRCFaceTracking;

namespace FaceBridge.ModuleHost.Loading;

/// <summary>An instantiated tracking module and the context it lives in.</summary>
internal sealed class LoadedModule
{
    private LoadedModule(ExtTrackingModule instance, Type type, ModuleLoadContext context)
    {
        Instance = instance;
        Type = type;
        Context = context;
    }

    public ExtTrackingModule Instance { get; }

    public Type Type { get; }

    public ModuleLoadContext Context { get; }

    /// <summary>
    /// Loads <paramref name="modulePath"/> and instantiates its single public non-abstract
    /// <see cref="ExtTrackingModule"/> subclass. Throws with a user-readable message on any problem.
    /// </summary>
    public static LoadedModule Load(string modulePath, Func<Type, ILogger> loggerFactory)
    {
        if (!File.Exists(modulePath))
        {
            throw new FileNotFoundException($"Module file not found: {modulePath}");
        }

        string directory = Path.GetDirectoryName(modulePath)!;
        AddNativeSearchDirectory(directory);

        var context = new ModuleLoadContext(directory);
        Assembly assembly = context.LoadFromAssemblyPath(modulePath);

        Type[] types;
        string loaderDetail = string.Empty;
        try
        {
            types = assembly.GetTypes();
        }
        catch (ReflectionTypeLoadException e)
        {
            var found = new List<Type>();
            foreach (Type? t in e.Types)
            {
                if (t != null)
                {
                    found.Add(t);
                }
            }

            types = found.ToArray();
            var detail = new StringBuilder();
            foreach (Exception? inner in e.LoaderExceptions)
            {
                if (inner != null)
                {
                    detail.Append(" | ").Append(inner.Message);
                }
            }

            loaderDetail = detail.ToString();
        }

        var candidates = new List<Type>();
        foreach (Type t in types)
        {
            if (t.IsClass && t.IsPublic && !t.IsAbstract && typeof(ExtTrackingModule).IsAssignableFrom(t))
            {
                candidates.Add(t);
            }
        }

        if (candidates.Count == 0)
        {
            throw new InvalidOperationException($"{Path.GetFileName(modulePath)} contains no public non-abstract ExtTrackingModule subclass.{loaderDetail}");
        }

        if (candidates.Count > 1)
        {
            throw new InvalidOperationException($"{Path.GetFileName(modulePath)} contains several ExtTrackingModule subclasses: {string.Join(", ", candidates)}.");
        }

        Type moduleType = candidates[0];
        ExtTrackingModule instance;
        try
        {
            instance = (ExtTrackingModule)Activator.CreateInstance(moduleType)!;
        }
        catch (TargetInvocationException e) when (e.InnerException != null)
        {
            throw new InvalidOperationException($"{moduleType.FullName} constructor threw: {e.InnerException}", e.InnerException);
        }

        instance.Logger = loggerFactory(moduleType);
        return new LoadedModule(instance, moduleType, context);
    }

    private static void AddNativeSearchDirectory(string directory)
    {
        if (OperatingSystem.IsWindows())
        {
            // Native modules often depend on further DLLs sitting next to them.
            SetDllDirectoryW(directory);
        }
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetDllDirectoryW(string lpPathName);
}
