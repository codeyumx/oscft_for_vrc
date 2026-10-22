using System;
using System.Collections.Generic;
using System.Collections.Immutable;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Runtime.Loader;
using FaceBridge.ModuleHost.Loading;
using Microsoft.Extensions.Logging.Abstractions;
using VRCFaceTracking;
using Xunit;

namespace FaceBridge.Tests;

/// <summary>
/// Loads real, unmodified VRCFT modules through the host's load context and checks that every type and member
/// they reference in VRCFaceTracking.Core / VRCFaceTracking.SDK exists, with the exact signature, in the compat assemblies.
/// </summary>
public class CompatTests
{
    private static readonly string[] CompatAssemblies = { "VRCFaceTracking.Core", "VRCFaceTracking.SDK" };

    public static IEnumerable<object[]> Fixtures()
    {
        foreach ((string zip, string dll, string type) in RealModules.Zips)
        {
            yield return new object[] { zip, dll, type };
        }
    }

    [Theory]
    [MemberData(nameof(Fixtures))]
    public void Real_module_instantiates_in_the_host_context_and_reports_support(string zip, string dll, string expectedType)
    {
        string? dir = RealModules.Extract(zip);
        if (dir == null)
        {
            return; // fixture not present on this machine
        }

        LoadedModule loaded = LoadedModule.Load(Path.Combine(dir, dll), _ => NullLogger.Instance);

        Assert.Equal(expectedType, loaded.Type.FullName);
        Assert.Same(loaded.Context, AssemblyLoadContext.GetLoadContext(loaded.Type.Assembly));
        Assert.NotNull(loaded.Instance.Logger);
        (bool eye, bool expression) = loaded.Instance.Supported;
        Assert.True(eye || expression);

        // Whatever versions the module asks for, the interface assemblies are the host's single copies.
        foreach (AssemblyName reference in loaded.Type.Assembly.GetReferencedAssemblies().Where(r => r.Name!.StartsWith("VRCFaceTracking.")))
        {
            Assembly resolved = loaded.Context.LoadFromAssemblyName(reference);
            Assert.Same(AssemblyLoadContext.Default, AssemblyLoadContext.GetLoadContext(resolved));
            Assert.Equal(new Version(6, 0, 0, 0), resolved.GetName().Version);
        }

        Assert.Same(typeof(ExtTrackingModule).Assembly, loaded.Context.LoadFromAssemblyName(new AssemblyName("VRCFaceTracking.SDK, Version=5.4.5.0")));
        Assert.Same(typeof(ExtTrackingModule), loaded.Context.LoadFromAssemblyName(new AssemblyName("VRCFaceTracking.Core, Version=5.2.3.0")).GetType("VRCFaceTracking.ExtTrackingModule"));
    }

    [Theory]
    [MemberData(nameof(Fixtures))]
    public void Every_type_and_member_reference_into_the_interface_assemblies_resolves(string zip, string dll, string _)
    {
        string? dir = RealModules.Extract(zip);
        if (dir == null)
        {
            return;
        }

        var problems = new List<string>();
        int checkedTypes = 0, checkedMembers = 0;
        using FileStream stream = File.OpenRead(Path.Combine(dir, dll));
        using var pe = new PEReader(stream);
        MetadataReader md = pe.GetMetadataReader();
        var names = new SignatureNames();

        foreach (TypeReferenceHandle handle in md.TypeReferences)
        {
            TypeReference tr = md.GetTypeReference(handle);
            if (!IsCompatScope(md, tr.ResolutionScope))
            {
                continue;
            }

            checkedTypes++;
            if (ResolveType(names.TypeName(md, handle)) == null)
            {
                problems.Add("type " + names.TypeName(md, handle));
            }
        }

        foreach (MemberReferenceHandle handle in md.MemberReferences)
        {
            MemberReference member = md.GetMemberReference(handle);
            if (member.Parent.Kind != HandleKind.TypeReference
                || !IsCompatScope(md, md.GetTypeReference((TypeReferenceHandle)member.Parent).ResolutionScope))
            {
                continue;
            }

            checkedMembers++;
            Type? owner = ResolveType(names.TypeName(md, member.Parent));
            string memberName = md.GetString(member.Name);
            string label = $"{names.TypeName(md, member.Parent)}::{memberName}";
            if (owner == null)
            {
                problems.Add("owner of " + label);
                continue;
            }

            if (member.GetKind() == MemberReferenceKind.Field)
            {
                string expected = member.DecodeFieldSignature(names, null);
                FieldInfo? field = owner.GetField(memberName, AllMembers);
                if (field == null || Describe(field.FieldType) != expected)
                {
                    problems.Add($"field {label} : {expected} (found {field?.FieldType})");
                }
            }
            else
            {
                MethodSignature<string> sig = member.DecodeMethodSignature(names, null);
                MethodBase[] candidates = memberName == ".ctor"
                    ? owner.GetConstructors(AllMembers)
                    : owner.GetMethods(AllMembers).Where(m => m.Name == memberName).ToArray();
                bool found = candidates.Any(m => Matches(m, sig));
                if (!found)
                {
                    problems.Add($"method {label}({string.Join(",", sig.ParameterTypes)}) : {sig.ReturnType}");
                }
            }
        }

        Assert.True(checkedTypes > 5 && checkedMembers > 10, $"suspiciously few references checked ({checkedTypes} types, {checkedMembers} members)");
        Assert.Empty(problems);
    }

    private const BindingFlags AllMembers = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static | BindingFlags.FlattenHierarchy;

    private static Type? ResolveType(string fullName)
    {
        foreach (string assemblyName in CompatAssemblies)
        {
            Assembly assembly = AssemblyLoadContext.Default.LoadFromAssemblyName(new AssemblyName(assemblyName));
            Type? type = assembly.GetType(fullName);
            if (type != null)
            {
                return type;
            }
        }

        return null;
    }

    private static bool IsCompatScope(MetadataReader md, EntityHandle scope)
    {
        if (scope.Kind == HandleKind.TypeReference)
        {
            return IsCompatScope(md, md.GetTypeReference((TypeReferenceHandle)scope).ResolutionScope);
        }

        return scope.Kind == HandleKind.AssemblyReference
            && CompatAssemblies.Contains(md.GetString(md.GetAssemblyReference((AssemblyReferenceHandle)scope).Name));
    }

    private static bool Matches(MethodBase method, MethodSignature<string> sig)
    {
        ParameterInfo[] parameters = method.GetParameters();
        if (parameters.Length != sig.ParameterTypes.Length || method.IsStatic == sig.Header.IsInstance)
        {
            return false;
        }

        for (int i = 0; i < parameters.Length; i++)
        {
            if (Describe(parameters[i].ParameterType) != sig.ParameterTypes[i])
            {
                return false;
            }
        }

        return method is ConstructorInfo || Describe(((MethodInfo)method).ReturnType) == sig.ReturnType;
    }

    /// <summary>Reflection type rendered the same way <see cref="SignatureNames"/> renders metadata types.</summary>
    private static string Describe(Type t)
    {
        if (t.IsByRef)
        {
            return Describe(t.GetElementType()!) + "&";
        }

        if (t.IsArray)
        {
            return Describe(t.GetElementType()!) + "[" + new string(',', t.GetArrayRank() - 1) + "]";
        }

        if (t.IsGenericType && !t.IsGenericTypeDefinition)
        {
            string def = t.GetGenericTypeDefinition().FullName!;
            return def + "<" + string.Join(",", t.GetGenericArguments().Select(Describe)) + ">";
        }

        return t.FullName!;
    }

    private sealed class SignatureNames : ISignatureTypeProvider<string, object?>
    {
        public string TypeName(MetadataReader md, EntityHandle handle) => handle.Kind == HandleKind.TypeReference
            ? GetTypeFromReference(md, (TypeReferenceHandle)handle, 0)
            : GetTypeFromDefinition(md, (TypeDefinitionHandle)handle, 0);

        public string GetPrimitiveType(PrimitiveTypeCode typeCode) => typeCode switch
        {
            PrimitiveTypeCode.Void => "System.Void",
            PrimitiveTypeCode.Boolean => "System.Boolean",
            PrimitiveTypeCode.Char => "System.Char",
            PrimitiveTypeCode.SByte => "System.SByte",
            PrimitiveTypeCode.Byte => "System.Byte",
            PrimitiveTypeCode.Int16 => "System.Int16",
            PrimitiveTypeCode.UInt16 => "System.UInt16",
            PrimitiveTypeCode.Int32 => "System.Int32",
            PrimitiveTypeCode.UInt32 => "System.UInt32",
            PrimitiveTypeCode.Int64 => "System.Int64",
            PrimitiveTypeCode.UInt64 => "System.UInt64",
            PrimitiveTypeCode.Single => "System.Single",
            PrimitiveTypeCode.Double => "System.Double",
            PrimitiveTypeCode.String => "System.String",
            PrimitiveTypeCode.Object => "System.Object",
            PrimitiveTypeCode.IntPtr => "System.IntPtr",
            PrimitiveTypeCode.UIntPtr => "System.UIntPtr",
            PrimitiveTypeCode.TypedReference => "System.TypedReference",
            _ => typeCode.ToString(),
        };

        public string GetTypeFromDefinition(MetadataReader reader, TypeDefinitionHandle handle, byte rawTypeKind)
        {
            TypeDefinition def = reader.GetTypeDefinition(handle);
            string name = reader.GetString(def.Name);
            if (!def.GetDeclaringType().IsNil)
            {
                return GetTypeFromDefinition(reader, def.GetDeclaringType(), 0) + "+" + name;
            }

            return def.Namespace.IsNil ? name : reader.GetString(def.Namespace) + "." + name;
        }

        public string GetTypeFromReference(MetadataReader reader, TypeReferenceHandle handle, byte rawTypeKind)
        {
            TypeReference tr = reader.GetTypeReference(handle);
            string name = reader.GetString(tr.Name);
            if (tr.ResolutionScope.Kind == HandleKind.TypeReference)
            {
                return GetTypeFromReference(reader, (TypeReferenceHandle)tr.ResolutionScope, 0) + "+" + name;
            }

            return tr.Namespace.IsNil ? name : reader.GetString(tr.Namespace) + "." + name;
        }

        public string GetTypeFromSpecification(MetadataReader reader, object? genericContext, TypeSpecificationHandle handle, byte rawTypeKind) =>
            reader.GetTypeSpecification(handle).DecodeSignature(this, genericContext);

        public string GetSZArrayType(string elementType) => elementType + "[]";

        public string GetArrayType(string elementType, ArrayShape shape) => elementType + "[" + new string(',', shape.Rank - 1) + "]";

        public string GetByReferenceType(string elementType) => elementType + "&";

        public string GetPointerType(string elementType) => elementType + "*";

        public string GetGenericInstantiation(string genericType, ImmutableArray<string> typeArguments) =>
            genericType + "<" + string.Join(",", typeArguments) + ">";

        public string GetGenericMethodParameter(object? genericContext, int index) => "!!" + index;

        public string GetGenericTypeParameter(object? genericContext, int index) => "!" + index;

        public string GetFunctionPointerType(MethodSignature<string> signature) => "fnptr";

        public string GetModifiedType(string modifier, string unmodifiedType, bool isRequired) => unmodifiedType;

        public string GetPinnedType(string elementType) => elementType;
    }
}
