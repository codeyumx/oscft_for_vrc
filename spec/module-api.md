# VRCFT module binary interface (compat spec)

FaceBridge runs unmodified VRCFaceTracking (VRCFT) tracking modules. This file is the **interface spec** the
compat assemblies in `dotnet/` are written from. It lists public names, signatures and constant values only,
taken from the public metadata of compiled VRCFT assemblies (`VRCFaceTracking.SDK` / `VRCFaceTracking.Core`
6.0.0.0, Apache-2.0) and from the references that real module binaries make into them. No VRCFT implementation
code is used; behaviour notes below come from the public module docs (https://docs.vrcft.io/docs/vrcft-software/vrcft-sdk/tracking-module)
or are FaceBridge's own definitions.

## Binding rules

- Modules reference two assemblies by simple name: `VRCFaceTracking.Core` (seen versions 5.2.3.0, 5.4.5.0) and,
  since the SDK split, `VRCFaceTracking.SDK` (seen 1.0.0.0, 5.4.5.0). Older modules reference everything
  through `VRCFaceTracking.Core`; newer ones reference `ExtTrackingModule`, `ModuleMetadata`, `ModuleState`
  through `VRCFaceTracking.SDK`. Both must load and resolve to **one** set of types, so `VRCFaceTracking.Core`
  carries `[TypeForwardedTo]` for the SDK types.
- The module host resolves those two names, `Microsoft.Extensions.Logging.Abstractions` and `FaceBridge.Sdk`
  to the host's own copies **regardless of the version the module asks for**. Everything else resolves from the
  module's own folder first, then the shared framework.
- Modules are compiled for net8.0 to net10.0; the host runs on net10.0.
- Modules also P/Invoke native DLLs shipped next to them: the module's folder must be on the native search path.

## Members real modules use

Collected from `FrameEyeOSC.dll` (built against VRCFT 5.2.3) and `PicoFacialDataModule.dll` (built against 5.4.5):

```
# Module FrameEyeOSC.dll
assemblyref System.Runtime 8.0.0.0
assemblyref System.Net.Primitives 8.0.0.0
assemblyref System.Net.Sockets 8.0.0.0
assemblyref System.Text.Json 8.0.0.0
assemblyref VRCFaceTracking.SDK 1.0.0.0
assemblyref Microsoft.Extensions.Logging.Abstractions 9.0.0.0
assemblyref System.Net.NetworkInformation 8.0.0.0
assemblyref System.Collections 8.0.0.0
assemblyref VRCFaceTracking.Core 5.2.3.0
assemblyref System.Memory 8.0.0.0
assemblyref System.Runtime.InteropServices 8.0.0.0
assemblyref System.Linq 8.0.0.0
assemblyref System.Threading.Thread 8.0.0.0
assemblyref System.Threading 8.0.0.0
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedExpressionShape
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedEyeData
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedTrackingData
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Types.Vector2
typeref [VRCFaceTracking.Core]VRCFaceTracking.UnifiedTracking
typeref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule
typeref [VRCFaceTracking.SDK]VRCFaceTracking.ModuleMetadata
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedExpressionShape::Weight (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedEyeData::_maxDilation (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedEyeData::_minDilation (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedEyeData::Left (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedEyeData::Right (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData::Gaze (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData::Openness (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData::PupilDiameter_MM (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedTrackingData::Eye (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedTrackingData::Shapes (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Types.Vector2::x (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Types.Vector2::y (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.UnifiedTracking::Data (Field)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::.ctor (Method)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::Logger (Field)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::ModuleInformation (Field)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::Teardown (Method)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ModuleMetadata::set_Name (Method)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ModuleMetadata::set_StaticImages (Method)
# Module PicoFacialDataModule.dll
assemblyref System.Runtime 10.0.0.0
assemblyref VRCFaceTracking.Core 5.4.5.0
assemblyref VRCFaceTracking.SDK 5.4.5.0
assemblyref System.Net.Primitives 10.0.0.0
assemblyref System.Net.Sockets 10.0.0.0
assemblyref System.Security.Cryptography 10.0.0.0
assemblyref System.Runtime.Numerics 10.0.0.0
assemblyref Microsoft.Extensions.Logging.Abstractions 10.0.0.0
assemblyref System.Collections 10.0.0.0
assemblyref System.Threading.Thread 10.0.0.0
assemblyref System.Threading 10.0.0.0
assemblyref System.Text.Json 10.0.0.0
assemblyref System.Memory 10.0.0.0
assemblyref System.Linq 10.0.0.0
assemblyref System.Net.NameResolution 10.0.0.0
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedExpressionShape
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedEyeData
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedTrackingData
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Expressions.UnifiedExpressions
typeref [VRCFaceTracking.Core]VRCFaceTracking.Core.Types.Vector2
typeref [VRCFaceTracking.Core]VRCFaceTracking.UnifiedTracking
typeref [VRCFaceTracking.SDK]VRCFaceTracking.Core.Library.ModuleState
typeref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule
typeref [VRCFaceTracking.SDK]VRCFaceTracking.ModuleMetadata
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedExpressionShape::Weight (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedEyeData::Left (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedEyeData::Right (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData::Gaze (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData::Openness (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData::PupilDiameter_MM (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedTrackingData::Eye (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Params.Data.UnifiedTrackingData::Shapes (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Types.Vector2::x (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.Core.Types.Vector2::y (Field)
memberref [VRCFaceTracking.Core]VRCFaceTracking.UnifiedTracking::Data (Field)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::.ctor (Method)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::Logger (Field)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::ModuleInformation (Field)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::Status (Field)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ExtTrackingModule::Teardown (Method)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ModuleMetadata::get_StaticImages (Method)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ModuleMetadata::set_Active (Method)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ModuleMetadata::set_Name (Method)
memberref [VRCFaceTracking.SDK]VRCFaceTracking.ModuleMetadata::set_StaticImages (Method)
```

## Module-facing public surface to implement

Exact names, member kinds, field types and enum values. Field vs property matters (modules compile to `ldfld`
vs `call get_X`), as do struct vs class and enum numeric values (compiled in as constants). Host-internal types
of VRCFT (parameter graph, OSC, mutator, IPC, services) are not part of the module interface and are not
implemented; `UnifiedTracking.Mutator` / `AllParameters*` / `UpdateData*` are omitted for that reason.

## Assembly `VRCFaceTracking.SDK`

```
enum VRCFaceTracking.Core.Library.ModuleState : System.IComparable, System.ISpanFormattable, System.IFormattable, System.IConvertible
  Uninitialized = -1
  Idle = 0
  Active = 1

abstract class VRCFaceTracking.ExtTrackingModule
  field VRCFaceTracking.Core.Library.ModuleState Status
  field Microsoft.Extensions.Logging.ILogger Logger
  field VRCFaceTracking.ModuleMetadata ModuleInformation
  property virtual System.ValueTuple`2[System.Boolean,System.Boolean] Supported { get; }
  method abstract System.ValueTuple`2[System.Boolean,System.Boolean] Initialize(System.Boolean eyeAvailable, System.Boolean expressionAvailable)
  method abstract System.Void Update()
  method abstract System.Void Teardown()

struct VRCFaceTracking.ModuleMetadata
  field VRCFaceTracking.ModuleMetadata+ActiveChange OnActiveChange
  property System.Collections.Generic.List`1[System.IO.Stream] StaticImages { get; set; }
  property System.String Name { get; set; }
  property System.Boolean Active { get; set; }
  property System.Boolean UsingEye { get; set; }
  property System.Boolean UsingExpression { get; set; }

class VRCFaceTracking.ModuleMetadata+ActiveChange : System.MulticastDelegate, System.ICloneable, System.Runtime.Serialization.ISerializable
  ctor(System.Object object, System.IntPtr method)
  method virtual System.Void Invoke(System.Boolean state)
  method virtual System.IAsyncResult BeginInvoke(System.Boolean state, System.AsyncCallback callback, System.Object object)
  method virtual System.Void EndInvoke(System.IAsyncResult result)

static class VRCFaceTracking.ModuleNotifications
  field static const System.Int32 Id = 7400
  field static const System.String Name = VRCFT.UserNotification
  field static readonly Microsoft.Extensions.Logging.EventId EventId
  method static System.Void NotifyUser(Microsoft.Extensions.Logging.ILogger logger, Microsoft.Extensions.Logging.LogLevel level, System.String message)
  method static System.Boolean IsNotification(Microsoft.Extensions.Logging.EventId eventId)
```

## Assembly `VRCFaceTracking.Core`

```
forward VRCFaceTracking.Core.Library.ModuleState -> VRCFaceTracking.SDK

forward VRCFaceTracking.ExtTrackingModule -> VRCFaceTracking.SDK

forward VRCFaceTracking.ModuleMetadata -> VRCFaceTracking.SDK

forward VRCFaceTracking.ModuleMetadata+ActiveChange -> VRCFaceTracking.SDK

struct VRCFaceTracking.Core.Params.Data.UnifiedExpressionShape
  field System.Single Weight

class VRCFaceTracking.Core.Params.Data.UnifiedEyeData
  field VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData Left
  field VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData Right
  field System.Single _maxDilation
  field System.Single _minDilation
  field System.Single _leftDiameter
  field System.Single _rightDiameter
  ctor()
  method VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData Combined()
  method System.Void CopyPropertiesOf(VRCFaceTracking.Core.Params.Data.UnifiedEyeData data)
  method System.Void CopyTo(System.Byte[]& bytes)

struct VRCFaceTracking.Core.Params.Data.UnifiedHeadData
  field System.Single HeadYaw
  field System.Single HeadPitch
  field System.Single HeadRoll
  field System.Single HeadPosX
  field System.Single HeadPosY
  field System.Single HeadPosZ

struct VRCFaceTracking.Core.Params.Data.UnifiedSingleEyeData
  field VRCFaceTracking.Core.Types.Vector2 Gaze
  field System.Single PupilDiameter_MM
  field System.Single Openness
  ctor()

class VRCFaceTracking.Core.Params.Data.UnifiedTrackingData
  field VRCFaceTracking.Core.Params.Data.UnifiedEyeData Eye
  field VRCFaceTracking.Core.Params.Data.UnifiedExpressionShape[] Shapes
  field VRCFaceTracking.Core.Params.Data.UnifiedHeadData Head
  field static const System.Int32 SerializedLength = 102
  ctor()
  method System.Void CopyPropertiesOf(VRCFaceTracking.Core.Params.Data.UnifiedTrackingData data)
  method System.Void CopyTo(System.Byte[]& bytes)

enum VRCFaceTracking.Core.Params.Expressions.UnifiedExpressions : System.IComparable, System.ISpanFormattable, System.IFormattable, System.IConvertible
  EyeSquintRight = 0
  EyeSquintLeft = 1
  EyeWideRight = 2
  EyeWideLeft = 3
  BrowPinchRight = 4
  BrowPinchLeft = 5
  BrowLowererRight = 6
  BrowLowererLeft = 7
  BrowInnerUpRight = 8
  BrowInnerUpLeft = 9
  BrowOuterUpRight = 10
  BrowOuterUpLeft = 11
  NasalDilationRight = 12
  NasalDilationLeft = 13
  NasalConstrictRight = 14
  NasalConstrictLeft = 15
  CheekSquintRight = 16
  CheekSquintLeft = 17
  CheekPuffRight = 18
  CheekPuffLeft = 19
  CheekSuckRight = 20
  CheekSuckLeft = 21
  JawOpen = 22
  JawRight = 23
  JawLeft = 24
  JawForward = 25
  JawBackward = 26
  JawClench = 27
  JawMandibleRaise = 28
  MouthClosed = 29
  LipSuckUpperRight = 30
  LipSuckUpperLeft = 31
  LipSuckLowerRight = 32
  LipSuckLowerLeft = 33
  LipSuckCornerRight = 34
  LipSuckCornerLeft = 35
  LipFunnelUpperRight = 36
  LipFunnelUpperLeft = 37
  LipFunnelLowerRight = 38
  LipFunnelLowerLeft = 39
  LipPuckerUpperRight = 40
  LipPuckerUpperLeft = 41
  LipPuckerLowerRight = 42
  LipPuckerLowerLeft = 43
  MouthUpperUpRight = 44
  MouthUpperUpLeft = 45
  MouthUpperDeepenRight = 46
  MouthUpperDeepenLeft = 47
  NoseSneerRight = 48
  NoseSneerLeft = 49
  MouthLowerDownRight = 50
  MouthLowerDownLeft = 51
  MouthUpperRight = 52
  MouthUpperLeft = 53
  MouthLowerRight = 54
  MouthLowerLeft = 55
  MouthCornerPullRight = 56
  MouthCornerPullLeft = 57
  MouthCornerSlantRight = 58
  MouthCornerSlantLeft = 59
  MouthFrownRight = 60
  MouthFrownLeft = 61
  MouthStretchRight = 62
  MouthStretchLeft = 63
  MouthDimpleRight = 64
  MouthDimpleLeft = 65
  MouthRaiserUpper = 66
  MouthRaiserLower = 67
  MouthPressRight = 68
  MouthPressLeft = 69
  MouthTightenerRight = 70
  MouthTightenerLeft = 71
  TongueOut = 72
  TongueUp = 73
  TongueDown = 74
  TongueRight = 75
  TongueLeft = 76
  TongueRoll = 77
  TongueBendDown = 78
  TongueCurlUp = 79
  TongueSquish = 80
  TongueFlat = 81
  TongueTwistRight = 82
  TongueTwistLeft = 83
  SoftPalateClose = 84
  ThroatSwallow = 85
  NeckFlexRight = 86
  NeckFlexLeft = 87
  Max = 88

enum VRCFaceTracking.Core.Params.Expressions.UnifiedSimpleExpressions : System.IComparable, System.ISpanFormattable, System.IFormattable, System.IConvertible
  BrowUpRight = 0
  BrowUpLeft = 1
  BrowDownRight = 2
  BrowDownLeft = 3
  MouthSmileRight = 4
  MouthSmileLeft = 5
  MouthSadRight = 6
  MouthSadLeft = 7

static class VRCFaceTracking.Core.Params.Expressions.UnifiedSimplifier
  field static System.Collections.Generic.Dictionary`2[VRCFaceTracking.Core.Params.Expressions.UnifiedSimpleExpressions,System.Func`2[VRCFaceTracking.Core.Params.Data.UnifiedTrackingData,System.Single]] ExpressionMap

class VRCFaceTracking.Core.Types.Image
  field System.ValueTuple`2[System.Int32,System.Int32] ImageSize
  field System.Byte[] ImageData
  field System.Boolean SupportsImage
  ctor()

struct VRCFaceTracking.Core.Types.Vector2
  field System.Single x
  field System.Single y
  property static VRCFaceTracking.Core.Types.Vector2 zero { get; }
  ctor(System.Single x, System.Single y)
  method VRCFaceTracking.Core.Types.Vector2 PolarTo2DCartesian(System.Single r)
  method VRCFaceTracking.Core.Types.Vector2 FlipXCoordinates()
  method System.Single ToYaw()
  method System.Single ToPitch()

struct VRCFaceTracking.Core.Types.Vector3
  field System.Single x
  field System.Single y
  field System.Single z
  property static VRCFaceTracking.Core.Types.Vector3 zero { get; }
  ctor(System.Single x, System.Single y, System.Single z)
  method VRCFaceTracking.Core.Types.Vector3 FlipXCoordinates()
  method VRCFaceTracking.Core.Types.Vector3 PolarTo2DCartesian(System.Single r)

struct VRCFaceTracking.Core.Types.Vector4
  field System.Single w
  field System.Single x
  field System.Single y
  field System.Single z
  ctor(System.Single w, System.Single x, System.Single y, System.Single z)

static class VRCFaceTracking.Core.Utils
  field static const System.Int32 PROCESS_VM_READ = 16
  field static readonly System.Boolean HasAdmin
  field static readonly System.String UserAccessibleDataDirectory
  field static readonly System.String PersistentDataDirectory
  field static readonly System.String CustomLibsDirectory
  method static System.UInt32 TimeBeginPeriod(System.UInt32 uMilliseconds)
  method static System.UInt32 TimeEndPeriod(System.UInt32 uMilliseconds)
  method static System.IntPtr OpenProcess(System.Int32 dwDesiredAccess, System.Boolean bInheritHandle, System.Int32 dwProcessId)
  method static System.Boolean ReadProcessMemory(System.Int32 hProcess, System.IntPtr lpBaseAddress, System.Byte[] lpBuffer, System.Int32 dwSize, System.Int32& lpNumberOfBytesRead)
  method static System.Boolean DeleteFile(System.String lpFileName)
  method static System.UInt32 GetFileAttributes(System.String lpFileName)
  method static System.Int32 GetRandomFreePort()
  method static System.String GetRandomChars(System.Int32 num)
  method static System.Void KillAllProcessesOfName(System.String name)

class VRCFaceTracking.UnifiedTracking
  field static VRCFaceTracking.Core.Types.Image EyeImageData
  field static VRCFaceTracking.Core.Types.Image LipImageData
  field static VRCFaceTracking.Core.Params.Data.UnifiedTrackingData Data
  field static System.Action`1[VRCFaceTracking.Core.Params.Data.UnifiedTrackingData] OnUnifiedDataUpdated
  ctor()

class VRCFaceTracking.Utils
  field static const System.Int32 PROCESS_VM_READ = 16
  field static readonly System.Boolean HasAdmin
  field static readonly System.String UserAccessibleDataDirectory
  field static readonly System.String PersistentDataDirectory
  field static readonly System.String CustomLibsDirectory
  ctor()
  method static System.UInt32 TimeBeginPeriod(System.UInt32 uMilliseconds)
  method static System.UInt32 TimeEndPeriod(System.UInt32 uMilliseconds)
  method static System.IntPtr OpenProcess(System.Int32 dwDesiredAccess, System.Boolean bInheritHandle, System.Int32 dwProcessId)
  method static System.Boolean ReadProcessMemory(System.Int32 hProcess, System.IntPtr lpBaseAddress, System.Byte[] lpBuffer, System.Int32 dwSize, System.Int32& lpNumberOfBytesRead)
  method static System.Boolean DeleteFile(System.String lpFileName)
  method static System.UInt32 GetFileAttributes(System.String lpFileName)
```

## Behaviour (FaceBridge definitions, consistent with the public VRCFT module docs)

- Lifecycle: host instantiates the single public non-abstract `ExtTrackingModule` subclass in the module DLL
  (parameterless ctor), assigns `Logger` (an `ILogger` whose category is the module type name) and leaves
  `ModuleInformation` default, reads `Supported`, calls `Initialize(eyeAvailable, expressionAvailable)` once.
  The returned tuple says which of eye / expression the module claimed; claiming something not offered is
  ignored. If it claims nothing it is torn down. Otherwise `Update()` is called in a loop on a dedicated thread
  until teardown (modules pace themselves, typically with `Thread.Sleep`). `Teardown()` is called once.
- `Status` is polled by the host; `ModuleInformation.Name` and `StaticImages` are read after `Initialize`.
- Data: modules write into the static `UnifiedTracking.Data` (`Eye.Left/Right` gaze, openness, pupil diameter
  in mm, `Shapes[(int)UnifiedExpressions.X].Weight`, `Head`). `UnifiedTracking.Data.Shapes` has length
  `UnifiedExpressions.Max` (88). Gaze is a normalized direction vector component pair (x right, y up), openness
  0..1, weights 0..1.
- `UnifiedEyeData._maxDilation` / `_minDilation` hold the largest / smallest pupil diameter seen (mm), used to
  normalize pupil dilation; modules may write them directly. `_leftDiameter` / `_rightDiameter` are per-eye
  copies kept by the host.
- `UnifiedTrackingData.SerializedLength = 102` is a public constant value only; FaceBridge does not use VRCFT's
  serialization format.
- Notifications: a log entry whose `EventId` has `Id == 7400` or `Name == "VRCFT.UserNotification"` is a
  user notification (`ModuleNotifications`). Modules built against an SDK without `ModuleNotifications` use
  `new EventId(7400, "VRCFT.UserNotification")`.
- `Utils.UserAccessibleDataDirectory` / `PersistentDataDirectory` / `CustomLibsDirectory`: FaceBridge points
  them at its own data directory / the module's data directory / the modules directory.
