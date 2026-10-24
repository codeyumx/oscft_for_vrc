# Module host IPC protocol (v1)

The Rust host (`fb-core`) runs each module in its own process: `FaceBridge.ModuleHost` for .NET (VRCFT) modules, or
the module's own program for native modules (`spec/native-modules.md`). They talk over one loopback TCP connection.
This file is the contract every side implements (`crates/fb-protocol`, `dotnet/FaceBridge.ModuleHost/Ipc`,
`crates/fb-module-sdk`). Bump `PROTOCOL_VERSION` on any incompatible change.

## Process launch

The host binds `127.0.0.1:0`, then starts:

```
FaceBridge.ModuleHost.exe --connect 127.0.0.1:<port> --token <32 hex chars> --module <absolute path to module dll>
                          --data-dir <absolute path, module's writable data dir> --parent-pid <host pid>
```

A native module's executable gets the same arguments without `--module` (`spec/native-modules.md`).

- The child connects and sends `Hello` first. The host drops any connection whose first message is not a
  `Hello` with the right token and version within 10 s, and keeps listening until its own child connects.
- The child exits when the connection closes, when the parent process exits (`--parent-pid` watchdog), or after
  `Teardown`. The host additionally assigns children to a Windows Job Object with kill-on-close.
- The child must never write protocol data to stdout. It redirects `Console.Out`/`Console.Error` into `Log`
  messages before loading the module; the host also captures the child's stdout/stderr as raw log lines.
- Exit codes: 0 ok, 2 bad arguments, 3 connect/handshake failure, 4 module load failure, 5 module crashed
  (unhandled exception in `Initialize`/`Update`), 6 parent gone.

## Framing

Every message: `u32 length` (little-endian; counts the type byte + payload), `u8 type`, payload.
Maximum length 16 MiB; a larger length is a protocol error and closes the connection. Unknown types are a
protocol error.

Encodings (all little-endian): `bool` = u8 0/1; `str` = u32 byte length + UTF-8 bytes; `bytes` = u32 length
+ raw; `f32`/`i32`/`u16`/`u32`/`u64` fixed width; `level` = u8 log level using `Microsoft.Extensions.Logging.LogLevel`
numbering (0 Trace, 1 Debug, 2 Information, 3 Warning, 4 Error, 5 Critical).

## Probe mode

`FaceBridge.ModuleHost.exe --probe <dir>` inspects every `*.dll` in `<dir>` (metadata only, no code runs) and
prints one line of JSON to stdout, `{"modules":[{"dll":"X.dll","type":"Namespace.Type"}]}`, listing each public
non-abstract class deriving from `VRCFaceTracking.ExtTrackingModule`; exit 0. Used for zips without `module.json`.

## Child → host

| Type | Name | Payload | Notes |
|---|---|---|---|
| 0x01 | Hello | `u16 version`, `str token`, `u32 pid` | First message. |
| 0x02 | Loaded | `str module_type`, `bool supports_eye`, `bool supports_expression` | Assembly loaded, module instantiated, `Supported` read. |
| 0x03 | LoadFailed | `str reason` | Followed by exit code 4. |
| 0x04 | InitResult | `bool eye`, `bool expression`, `str name`, `u16 image_count`, `image_count × bytes` | Reply to `Init`. Images are the raw bytes of `ModuleInformation.StaticImages` streams (usually PNG). `name` empty if module left it unset. |
| 0x05 | Frame | see below | Sent after each `Update()` returns, at most once per millisecond. |
| 0x06 | Log | `level`, `i32 event_id`, `str category`, `str message`, `str exception` | `exception` empty when none. Notification log entries (event 7400) are sent as both `Log` and `Notify`. |
| 0x07 | Notify | `level`, `str key`, `str message` | Informational pop-up; never stored, fades by itself. |
| 0x08 | NotifyClear | `str key` | Remove this module's message (`NotifyMessage`) with `key`; empty = remove all of this module's messages. |
| 0x09 | Status | `i8 state` | `ModuleState` value (-1, 0, 1); sent when the polled `Status` field changes. |
| 0x0A | SettingsSchema | `str json` | The module declared settings via `FaceBridge.Sdk` (schema below). May be sent again if the module redeclares. |
| 0x0B | SettingsWrite | `str json` | The module changed its own setting values (object of key → value); host persists and shows them. |
| 0x0C | TeardownDone | – | `Teardown()` returned; child exits 0 right after. |
| 0x0D | NotifyMessage | `level`, `str key`, `str message` | A message: stays on the Home page until the user dismisses it or `NotifyClear` removes it. Same `key` replaces the previous one from this module; empty `key` = keyed by message text (dedupe). |

### Frame payload

```
u64 seq                      // increments per frame, starts at 1
u8  flags                    // bit0 eye data valid (module claimed eye), bit1 expression valid, bit2 head valid
f32 left_gaze_x, left_gaze_y, left_openness, left_pupil_diameter_mm
f32 right_gaze_x, right_gaze_y, right_openness, right_pupil_diameter_mm
f32 max_dilation, min_dilation, left_diameter, right_diameter
f32 head_yaw, head_pitch, head_roll, head_pos_x, head_pos_y, head_pos_z
u16 shape_count              // UnifiedExpressions.Max (88) today; host ignores extra, zero-fills missing
f32 shapes[shape_count]
```

The child sets flag bit2 when any head field is non-zero (VRCFT modules have no head claim). Bit0/bit1 follow
what the module claimed in `Initialize`; fields of unclaimed parts are still sent but the host ignores them.

## Host → child

| Type | Name | Payload | Notes |
|---|---|---|---|
| 0x81 | Init | `bool eye_available`, `bool expression_available` | Sent once after `Loaded` (and after `Settings` if the host has stored values). |
| 0x82 | Settings | `str json` | Current values (object of key → value). Sent before `Init` when stored values exist, and whenever the user changes a setting. |
| 0x83 | Teardown | – | Child calls `Teardown()`, replies `TeardownDone`, exits. Host kills the child if no `TeardownDone` within 5 s. |
| 0x84 | SettingsAction | `str key` | The user pressed the settings `button` field `key`. Only sent after `Init`. The SDK runs the module's handler on a thread-pool thread; unknown keys are ignored. |

## Settings schema JSON

```json
{
  "fields": [
    { "key": "port", "label": "UDP port", "description": "Port the headset sends to.", "type": "int",
      "default": 11112, "min": 1, "max": 65535 },
    { "key": "smoothing", "label": "Smoothing", "type": "float", "default": 0.5, "min": 0, "max": 1, "step": 0.05 },
    { "key": "verbose", "label": "Verbose logging", "type": "bool", "default": false },
    { "key": "host", "label": "Headset address", "type": "string", "default": "" },
    { "key": "mode", "label": "Mode", "type": "choice", "default": "auto",
      "options": [ { "value": "auto", "label": "Automatic" }, { "value": "manual", "label": "Manual" } ] }
  ]
}
```

`type` is one of `int`, `float`, `bool`, `string`, `choice`, `text`, `button`. `description`, `min`, `max`, `step`
optional. Keys are unique per module. Values objects map key → JSON value of the field's type; the host drops
unknown keys and values of the wrong type, clamps numbers to `min`/`max`, and fills missing keys with `default`.

- `text` is read-only display text owned by the module (status lines, a pairing code to type on the headset).
  `default` is the text at declaration time; the module changes it with `SettingsWrite`. The user cannot change
  it, the host does not store it in `settings.json`, does not include it in `Settings`, and resets it to `default`
  when the module (re)declares. Newlines are shown.
- `button` has no `default` and no value; pressing it sends `SettingsAction`.
- A field whose `type` the host does not know is skipped (not shown, no value), so a newer SDK never makes the
  whole form unusable.

Example, a "Pair by code" form:

```json
{ "fields": [
  { "key": "pair_code", "label": "Pairing code", "type": "string", "default": "" },
  { "key": "pair", "label": "Pair by code", "type": "button" },
  { "key": "pair_status", "label": "Status", "type": "text", "default": "Not paired" }
] }
```

## Test vectors

Both implementations must encode these exactly (and decode them back):

| Message | Bytes (hex) |
|---|---|
| `Hello { version: 1, token: "0123456789abcdef0123456789abcdef", pid: 4242 }` | `2b 00 00 00 01 01 00 20 00 00 00 30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66 30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66 92 10 00 00` |
| `SettingsAction { key: "pair" }` | `09 00 00 00 84 04 00 00 00 70 61 69 72` |
| `Init { eye_available: true, expression_available: false }` | `03 00 00 00 81 01 00` |
| `Notify { level: Warning(3), key: "no-headset", message: "No headset answered" }` | `27 00 00 00 07 03 0a 00 00 00 6e 6f 2d 68 65 61 64 73 65 74 13 00 00 00 4e 6f 20 68 65 61 64 73 65 74 20 61 6e 73 77 65 72 65 64` |
| `Frame { seq: 7, flags: EYE\|EXPRESSION, left: (0.25,-0.5,1.0,3.5), right: (0,0,0.75,4.0), max_dilation: 6, min_dilation: 2, left_diameter: 3.5, right_diameter: 4, head: 0×6, shapes: [0, 0.5, 1] }` | `60 00 00 00 05 07 00 00 00 00 00 00 00 03 00 00 80 3e 00 00 00 bf 00 00 80 3f 00 00 60 40 00 00 00 00 00 00 00 00 00 00 40 3f 00 00 80 40 00 00 c0 40 00 00 00 40 00 00 60 40 00 00 80 40 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 03 00 00 00 00 00 00 00 00 3f 00 00 80 3f` |
