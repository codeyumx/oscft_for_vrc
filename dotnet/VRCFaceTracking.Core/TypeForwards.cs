using System.Runtime.CompilerServices;
using VRCFaceTracking;
using VRCFaceTracking.Core.Library;

// Older modules reference the SDK types through VRCFaceTracking.Core; newer ones through VRCFaceTracking.SDK.
// Forwarding makes both resolve to the same runtime types.
[assembly: TypeForwardedTo(typeof(ModuleState))]
[assembly: TypeForwardedTo(typeof(ExtTrackingModule))]
[assembly: TypeForwardedTo(typeof(ModuleMetadata))]
// The nested ModuleMetadata.ActiveChange delegate follows its enclosing type's forwarder; C# cannot forward nested types directly.
