using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Text.Json;

namespace FaceBridge.ModuleHost;

/// <summary>
/// <c>--probe &lt;dir&gt;</c>: finds ExtTrackingModule subclasses by reading assembly metadata only; no module code runs.
/// </summary>
internal static class Probe
{
    private const string BaseTypeName = "VRCFaceTracking.ExtTrackingModule";

    private readonly record struct TypeKey(string Assembly, string FullName);

    private sealed record TypeInfo(string Dll, TypeKey Key, TypeKey? Base, bool IsPublicClass);

    public static int Run(string directory, TextWriter output)
    {
        List<(string Dll, string Type)> modules = Find(directory);
        using var stream = new MemoryStream();
        using (var json = new Utf8JsonWriter(stream))
        {
            json.WriteStartObject();
            json.WriteStartArray("modules");
            foreach ((string dll, string type) in modules)
            {
                json.WriteStartObject();
                json.WriteString("dll", dll);
                json.WriteString("type", type);
                json.WriteEndObject();
            }

            json.WriteEndArray();
            json.WriteEndObject();
        }

        output.WriteLine(System.Text.Encoding.UTF8.GetString(stream.GetBuffer(), 0, (int)stream.Length));
        return ExitCodes.Ok;
    }

    /// <summary>Every public non-abstract class in <paramref name="directory"/>'s DLLs that derives from ExtTrackingModule.</summary>
    public static List<(string Dll, string Type)> Find(string directory)
    {
        var all = new Dictionary<TypeKey, TypeInfo>();
        var files = Directory.GetFiles(directory, "*.dll").OrderBy(f => f, StringComparer.OrdinalIgnoreCase).ToList();
        foreach (string file in files)
        {
            foreach (TypeInfo info in ReadTypes(file))
            {
                all[info.Key] = info;
            }
        }

        var result = new List<(string, string)>();
        foreach (TypeInfo info in all.Values.OrderBy(i => i.Dll, StringComparer.OrdinalIgnoreCase).ThenBy(i => i.Key.FullName, StringComparer.Ordinal))
        {
            if (info.IsPublicClass && DerivesFromModule(info, all))
            {
                result.Add((info.Dll, info.Key.FullName));
            }
        }

        return result;
    }

    private static bool DerivesFromModule(TypeInfo start, Dictionary<TypeKey, TypeInfo> all)
    {
        TypeKey? current = start.Base;
        for (int depth = 0; depth < 64 && current is { } key; depth++)
        {
            if (key.FullName == BaseTypeName && key.Assembly is "VRCFaceTracking.SDK" or "VRCFaceTracking.Core")
            {
                return true;
            }

            current = all.TryGetValue(key, out TypeInfo? next) ? next.Base : null;
        }

        return false;
    }

    private static List<TypeInfo> ReadTypes(string file)
    {
        var infos = new List<TypeInfo>();
        try
        {
            using FileStream stream = File.OpenRead(file);
            using var pe = new PEReader(stream);
            if (!pe.HasMetadata)
            {
                return infos;
            }

            MetadataReader md = pe.GetMetadataReader();
            if (!md.IsAssembly)
            {
                return infos;
            }

            string assemblyName = md.GetString(md.GetAssemblyDefinition().Name);
            string dll = Path.GetFileName(file);
            foreach (TypeDefinitionHandle handle in md.TypeDefinitions)
            {
                TypeDefinition def = md.GetTypeDefinition(handle);
                if (!def.GetDeclaringType().IsNil)
                {
                    continue; // nested types are not module entry points
                }

                TypeAttributes attrs = def.Attributes;
                bool publicClass = (attrs & TypeAttributes.VisibilityMask) == TypeAttributes.Public
                    && (attrs & TypeAttributes.Interface) == 0
                    && (attrs & TypeAttributes.Abstract) == 0;
                var key = new TypeKey(assemblyName, FullName(md, def.Namespace, def.Name));
                infos.Add(new TypeInfo(dll, key, BaseKey(md, def.BaseType), publicClass));
            }
        }
        catch (Exception e) when (e is BadImageFormatException or IOException or InvalidOperationException)
        {
            // Not a managed assembly (native DLL) or unreadable: nothing to report.
        }

        return infos;
    }

    private static string FullName(MetadataReader md, StringHandle ns, StringHandle name)
    {
        string n = md.GetString(name);
        return ns.IsNil ? n : md.GetString(ns) + "." + n;
    }

    private static TypeKey? BaseKey(MetadataReader md, EntityHandle handle)
    {
        if (handle.IsNil)
        {
            return null;
        }

        if (handle.Kind == HandleKind.TypeSpecification)
        {
            // Generic base class: the first entry after GENERICINST + CLASS is the generic type definition.
            BlobReader blob = md.GetBlobReader(md.GetTypeSpecification((TypeSpecificationHandle)handle).Signature);
            if (blob.ReadByte() != (byte)SignatureTypeCode.GenericTypeInstance)
            {
                return null;
            }

            blob.ReadByte();
            return BaseKey(md, blob.ReadTypeHandle());
        }

        if (handle.Kind == HandleKind.TypeDefinition)
        {
            TypeDefinition def = md.GetTypeDefinition((TypeDefinitionHandle)handle);
            return new TypeKey(md.GetString(md.GetAssemblyDefinition().Name), FullName(md, def.Namespace, def.Name));
        }

        if (handle.Kind == HandleKind.TypeReference)
        {
            TypeReference tr = md.GetTypeReference((TypeReferenceHandle)handle);
            if (tr.ResolutionScope.Kind == HandleKind.AssemblyReference)
            {
                AssemblyReference ar = md.GetAssemblyReference((AssemblyReferenceHandle)tr.ResolutionScope);
                return new TypeKey(md.GetString(ar.Name), FullName(md, tr.Namespace, tr.Name));
            }
        }

        return null;
    }
}
