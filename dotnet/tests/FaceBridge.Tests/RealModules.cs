using System;
using System.IO;
using System.IO.Compression;

namespace FaceBridge.Tests;

/// <summary>Real VRCFT module binaries used as compatibility fixtures (read-only, extracted to temp).</summary>
internal static class RealModules
{
    public static readonly (string Zip, string Dll, string Type)[] Zips =
    {
        ("FrameEyeOSC_1.0.1-claim-with-packets.zip", "FrameEyeOSC.dll", "FrameEyeOSC.FrameEyeOscModule"),
        ("Pico4FaceTracking-VRCFT.zip", "PicoFacialDataModule.dll", "PicoFacialDataModule.PicoFacialDataModule"),
    };

    /// <summary>Extracts test-modules/&lt;zip&gt; to a temp folder; null when the fixture is absent.</summary>
    public static string? Extract(string zipName)
    {
        string zip = Path.Combine(TestPaths.RepoRoot, "test-modules", zipName);
        if (!File.Exists(zip))
        {
            return null;
        }

        string dir = TestPaths.NewTempDir(Path.GetFileNameWithoutExtension(zipName));
        ZipFile.ExtractToDirectory(zip, dir);
        return dir;
    }
}
