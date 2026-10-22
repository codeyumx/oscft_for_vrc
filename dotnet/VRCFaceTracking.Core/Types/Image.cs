namespace VRCFaceTracking.Core.Types;

/// <summary>Holder for a module-provided camera image. FaceBridge keeps the object but does not display it.</summary>
public class Image
{
    public (int, int) ImageSize;
    public byte[] ImageData = System.Array.Empty<byte>();
    public bool SupportsImage;

    public Image()
    {
    }
}
