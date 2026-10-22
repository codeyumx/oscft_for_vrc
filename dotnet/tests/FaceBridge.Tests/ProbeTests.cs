using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Text.Json.Nodes;
using FaceBridge.ModuleHost;
using Xunit;

namespace FaceBridge.Tests;

public class ProbeTests
{
    [Fact]
    public void Finds_the_module_type_by_metadata_and_ignores_compat_and_sdk_assemblies()
    {
        string dir = TestPaths.NewTempDir("probe");
        TestPaths.CopyDirectory(TestPaths.ProjectOutput("TestModule"), dir);
        // Even if a module zip ships its own copies of the interface assemblies they are not modules.
        File.Copy(Path.Combine(TestPaths.ProjectOutput("VRCFaceTracking.SDK"), "VRCFaceTracking.SDK.dll"), Path.Combine(dir, "VRCFaceTracking.SDK.dll"), true);
        File.Copy(Path.Combine(TestPaths.ProjectOutput("VRCFaceTracking.Core"), "VRCFaceTracking.Core.dll"), Path.Combine(dir, "VRCFaceTracking.Core.dll"), true);

        Assert.Equal(new[] { ("TestModule.dll", "TestModule.TestTrackingModule") }, Probe.Find(dir).ToArray());
    }

    [Fact]
    public void Prints_one_json_line()
    {
        string dir = TestPaths.NewTempDir("probe-json");
        TestPaths.CopyDirectory(TestPaths.ProjectOutput("TestModule"), dir);
        var writer = new StringWriter();
        Assert.Equal(0, Probe.Run(dir, writer));

        string line = writer.ToString().TrimEnd();
        Assert.DoesNotContain('\n', line);
        JsonNode? doc = JsonNode.Parse(line);
        Assert.Equal("TestModule.TestTrackingModule", (string?)doc!["modules"]![0]!["type"]);
        Assert.Equal("TestModule.dll", (string?)doc["modules"]![0]!["dll"]);
    }

    [Fact]
    public void Directory_without_modules_gives_empty_list()
    {
        string dir = TestPaths.NewTempDir("probe-empty");
        File.WriteAllText(Path.Combine(dir, "native.dll"), "not a managed assembly");
        var writer = new StringWriter();
        Assert.Equal(0, Probe.Run(dir, writer));
        Assert.Equal("{\"modules\":[]}", writer.ToString().TrimEnd());
    }

    [Fact]
    public void Finds_the_real_zipped_modules()
    {
        foreach ((string zip, string dll, string type) in RealModules.Zips)
        {
            string? dir = RealModules.Extract(zip);
            if (dir == null)
            {
                continue;
            }

            Assert.Equal(new[] { (dll, type) }, Probe.Find(dir).ToArray());
        }
    }
}
