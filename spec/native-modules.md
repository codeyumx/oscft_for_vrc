# Native modules

A native module is a program that speaks the module IPC protocol (`spec/ipc-protocol.md`) itself, instead of a
.NET DLL run by `FaceBridge.ModuleHost`. Native modules only run in OSCFT; VRCFaceTracking cannot load them.
.NET (VRCFT) modules keep working unchanged, and the app keeps the module host built in for them.

When only native modules are installed, OSCFT never locates, extracts or starts the .NET module host, so no .NET
runtime is needed and no .NET processes run.

## module.json

A native module's `module.json` names its program with `Executable` instead of `DllFileName`:

```json
{
  "ModuleId": "5d1c0f4e-2a7b-4c1e-9a43-6f0d2b8e7c11",
  "ModuleName": "Demo (native)",
  "Version": "1.0",
  "Executable": "demo_module"
}
```

- `Executable` is a plain file name in the module folder (no path separators). On Windows `<name>.exe` is used when
  it exists, so one zip can carry both a Linux `demo_module` and a Windows `demo_module.exe`.
- Setting both `Executable` and `DllFileName` is an install error.
- All other fields (`ModuleId`, `ModuleName`, `Version`, `DownloadUrl`, ...) mean the same as for .NET modules,
  including the update check. When `ModuleName` is missing, the executable's file stem is used.
- Install never runs `FaceBridge.ModuleHost --probe` for a native module. On Linux and macOS the executable is
  marked executable (zips do not keep the permission bit).

## Launch

The engine starts the executable with the module folder as working directory and the launch arguments of
`spec/ipc-protocol.md` without `--module`:

```
<executable> --connect 127.0.0.1:<port> --token <32 hex chars> --data-dir <absolute path> --parent-pid <host pid>
```

`FACEBRIDGE_USER_DATA_DIR` is set as for the module host. From there the session is exactly the protocol of
`spec/ipc-protocol.md`: `Hello`, `Loaded` (or `LoadFailed`), stored `Settings`, `Init`, `InitResult`, frames,
`Teardown`/`TeardownDone`, and the same exit codes. Frames carry Unified Expressions data (`fb-tracking`), so
native and .NET modules merge, calibrate and map to OSC the same way.

## Rust SDK (`crates/fb-module-sdk`)

`fb_module_sdk::run(load)` implements the child side; a module implements `Module`:

| Method | Called | Purpose |
|---|---|---|
| `supported()` | once, after `load` | parts the module can provide (`Loaded`) |
| `initialize(ctx, available)` | on `Init`, module thread | connect to the device; return `Init { claims, name, images }` |
| `run(ctx)` | after `initialize` when something was claimed | send frames until `ctx.is_stopping()` |
| `teardown(ctx)` | once, after `run` returned, when OSCFT stops an initialized module | release the device |

`Context`: `send_frame` (flags from the granted claims; head flag when any head field is non-zero; frames at least
1 ms apart), `sleep` (wakes early on stop), `log`, `notify` (transient pop-up), `message` (stays on Home until
dismissed), `clear_message`, `set_status`,
`declare_settings`, `write_settings`, `settings`, `try_event`/`wait_event` (`SettingsChanged`, `Action(key)`),
`data_dir`.

Behaviour matching `FaceBridge.ModuleHost`:

- A `load` error or panic sends `LoadFailed` and exits 4.
- A panic in `initialize` or `run` (not during a stop) is logged as `Critical` and exits 5.
- On `Teardown` or when the connection closes, the SDK requests a stop, waits up to 4 s for `run` and `teardown`,
  replies `TeardownDone` (on `Teardown`) and exits 0. Teardown before `Init` never calls the module.
- `--parent-pid` is validated but not watched: when OSCFT exits, its end of the connection closes and the module
  stops.

Example: `crates/fb-module-sdk/examples/demo_module.rs` (synthetic blinking eyes, jaw and smile, with a settings form
using a slider, toggle, dropdown, button and status text).
