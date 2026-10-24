# Rust crate contracts

Crate graph (arrows = depends on):

```
fb-tracking  <-  fb-protocol  <-  fb-core  ->  fb-osc  ->  fb-tracking
fb-tracking  <-  fb-mutate    <-  fb-core
                                     ^
                     apps/facebridge (Tauri)   crates/facebridge-cli (headless)
```

Names below are the contract between crates; private internals are up to each crate. All snapshot/view types
derive `serde::Serialize` with `#[serde(rename_all = "camelCase")]` (the Tauri UI consumes them as JSON).
Errors: `anyhow::Result` at the `fb-core` boundary, typed errors inside.

## fb-protocol

Wire types and blocking encode/decode for `spec/ipc-protocol.md`.

```rust
pub const PROTOCOL_VERSION: u16 = 1;
pub enum ChildMessage { Hello { version: u16, token: String, pid: u32 }, Loaded { module_type: String, supports_eye: bool, supports_expression: bool },
    LoadFailed { reason: String }, InitResult { eye: bool, expression: bool, name: String, images: Vec<Vec<u8>> },
    Frame(Box<FrameMessage>), Log { level: LogLevel, event_id: i32, category: String, message: String, exception: String },
    Notify { level: LogLevel, key: String, message: String }, NotifyMessage { level: LogLevel, key: String, message: String },
    NotifyClear { key: String }, Status { state: i8 },
    SettingsSchema { json: String }, SettingsWrite { json: String }, TeardownDone }
pub enum HostMessage { Init { eye_available: bool, expression_available: bool }, Settings { json: String }, Teardown }
pub struct FrameMessage { pub seq: u64, pub flags: FrameFlags, pub frame: fb_tracking::TrackingFrame }
pub enum LogLevel { Trace = 0, Debug, Information, Warning, Error, Critical }
pub fn write_host_message(w: &mut impl Write, m: &HostMessage) -> io::Result<()>;
pub fn read_child_message(r: &mut impl Read, buf: &mut Vec<u8>) -> Result<ChildMessage, ProtocolError>;
// plus the mirror pair (write_child_message / read_host_message) used by tests and fake children
```

## fb-osc

Everything VRChat-facing: OSC encoding, parameter mapping (`spec/osc-output.md`), avatar parameter discovery,
sending. No knowledge of modules.

```rust
pub struct OscConfig {            // Serialize + Deserialize + Clone, persisted by fb-core
    pub send_addr: SocketAddr,    // default 127.0.0.1:9000
    pub listen_port: u16,         // default 9001 (legacy /avatar/change + replies); 0 = OSCQuery only
    pub rate_hz: u32,             // output tick rate, default 60
    pub oscquery: bool,           // advertise + query via OSCQuery, default true
}
pub struct Claims { pub eye: bool, pub expression: bool }

pub struct OscOutput { .. }       // Send + Sync; owns sockets and its threads; stops on drop
impl OscOutput {
    pub fn start(config: OscConfig, on_event: impl Fn(OscEvent) + Send + Sync + 'static) -> io::Result<Self>;
    pub fn submit(&self, frame: &TrackingFrame, claims: Claims); // latest wins; never blocks on the network
    pub fn status(&self) -> OscStatus;
    pub fn avatar(&self) -> Option<AvatarInfo>;
}
pub enum OscEvent { AvatarChanged(Option<AvatarInfo>), StatusChanged(OscStatus) }
pub struct AvatarInfo { pub id: String, pub name: Option<String>, pub parameter_count: usize, pub matched_parameter_count: usize, pub source: AvatarSource }
pub enum AvatarSource { OscQuery, ConfigFile }
pub struct OscStatus { pub send_addr: SocketAddr, pub sending: bool, pub messages_per_second: u32, pub last_error: Option<String>, pub vrchat_detected: bool }
```

## fb-core

```rust
pub struct EngineConfig {         // persisted as <data_dir>/config.json
    pub osc: fb_osc::OscConfig,
    pub module_host: Option<PathBuf>,   // FaceBridge.ModuleHost(.exe); None = discover (see below)
    pub dotnet_root: Option<PathBuf>,   // passed to children as DOTNET_ROOT when set
    pub modules: Vec<ModuleEntry>,      // install order = priority; { id, enabled, options }
    pub mutation: fb_mutate::MutationConfig,   // calibration, correctors, adjustment, filter (spec/mutation.md)
}
pub struct ModuleOptions {        // FaceBridge's options for every module, ModuleEntry.options (camelCase in JSON)
    pub allow_notifications: bool,      // default true; false = Notify only goes to the log (category "notification")
    pub require_data: bool,             // default true; startup live-data check, see "Startup live-data check"
    pub data_timeout_secs: u32,         // default 10; the check waits this long (clamped to 1..=120, also on set_module_options)
}
pub fn default_data_dir() -> PathBuf;   // %APPDATA%\FaceBridge (dirs::config_dir()/FaceBridge elsewhere)

pub struct Engine { .. }           // shared as Arc<Engine>
impl Engine {
    pub fn start(data_dir: PathBuf) -> anyhow::Result<Arc<Engine>>;   // load config, start OSC, start enabled modules
    pub fn subscribe(&self) -> crossbeam_channel::Receiver<EngineEvent>;
    pub fn snapshot(&self) -> EngineSnapshot;
    pub fn latest_frame(&self) -> (TrackingFrame, fb_osc::Claims);   // mutated output; see "fb-mutate and the mutation API"
    pub fn install_module(&self, path: &Path) -> anyhow::Result<ModuleId>;   // .zip or folder
    pub fn import_vrcft_modules(&self) -> anyhow::Result<Vec<ModuleId>>;     // copies %APPDATA%\VRCFaceTracking\CustomLibs\*
    pub fn uninstall_module(&self, id: &str) -> anyhow::Result<()>;
    pub fn set_module_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<()>;
    pub fn set_module_order(&self, ids: &[String]) -> anyhow::Result<()>;
    pub fn restart_modules(&self) -> anyhow::Result<()>;
    pub fn module_settings(&self, id: &str) -> Option<ModuleSettingsView>;     // { schema: serde_json::Value, values: serde_json::Value }
    pub fn set_module_settings(&self, id: &str, values: serde_json::Value) -> anyhow::Result<()>; // partial object, validated per schema; `text` fields are not user-writable
    pub fn set_module_options(&self, id: &str, options: ModuleOptions) -> anyhow::Result<()>;  // persisted; turning notifications off clears the module's current ones
    pub fn press_module_button(&self, id: &str, key: &str) -> anyhow::Result<()>;              // sends SettingsAction; error if no such button or the module is not running
    pub fn module_image(&self, id: &str, index: usize) -> Option<Vec<u8>>;
    pub fn dismiss_notification(&self, id: u64);
    pub fn set_osc_config(&self, osc: fb_osc::OscConfig) -> anyhow::Result<()>;
    pub fn shutdown(&self);   // tears down all modules (Teardown, 5 s, then kill), stops OSC; idempotent
}

pub enum EngineEvent { ModulesChanged, NotificationsChanged, Notice(NotificationView), Log(LogEntry), AvatarChanged, OscStatusChanged, MutationChanged }
pub struct EngineSnapshot { pub modules: Vec<ModuleView>, pub notifications: Vec<NotificationView>,
    pub avatar: Option<fb_osc::AvatarInfo>, pub osc: fb_osc::OscStatus, pub osc_config: fb_osc::OscConfig,
    pub logs: Vec<LogEntry> /* ring buffer, newest last, max 1000 */ }
pub struct ModuleView { pub id: String, pub name: String, pub version: Option<String>, pub author: Option<String>,
    pub description: Option<String>, pub usage: Option<String>, pub page_url: Option<String>,
    pub source_repo: Option<SourceRepo> /* from module.json DownloadUrl, else ModulePageUrl: Github { path: "owner/repo" }
        or Gitlab { host, path: "group/sub/project" } (gitlab.com or a gitlab.* host); JSON {"forge":"github"|"gitlab",...} */,
    pub options: ModuleOptions, pub enabled: bool, pub state: ModuleRunState, pub supports: Claims, pub claims: Claims,
    pub status: Option<i8>, pub last_error: Option<String>, pub has_settings: bool,
    pub settings_rev: u64 /* changes when the setting values change (user, module, redeclaration) */, pub image_count: usize }
pub enum ModuleRunState { Stopped, Starting, Initializing, Running, Unclaimed, NoData, Failed, Exited }   // serialized lowercase
pub struct NotificationView { pub id: u64, pub module_id: String, pub module_name: String, pub level: String /* "information"|"warning"|"error"|... */,
    pub key: String, pub message: String, pub time_ms: u64 }
pub struct LogEntry { pub time_ms: u64, pub module_id: Option<String>, pub level: String, pub category: String, pub message: String }
```

### Module storage

```
<data_dir>/config.json
<data_dir>/modules/<id>/            module files as shipped + module.json (VRCFT TrackingModuleMetadata format:
                                    ModuleId, ModuleName, Version, AuthorName, ModuleDescription, UsageInstructions,
                                    ModulePageUrl, DownloadUrl, DllFileName, ...; native modules: Executable instead of
                                    DllFileName, spec/native-modules.md); id = ModuleId GUID, or a new UUID for local zips
<data_dir>/module-data/<id>/        writable data dir passed as --data-dir; settings.json = current values (without `text` fields)
```

Zip install: extract to a temp dir. Native module (`module.json` `Executable`): the executable must exist and is
marked executable on Unix; no probe. .NET module: DLL = `module.json` `DllFileName` when present, otherwise run
`FaceBridge.ModuleHost --probe <dir>` (stdout JSON `{"modules":[{"dll":"X.dll","type":"Ns.Type"}]}`) and take
the single hit (error if none or several). Zip-slip safe.

### Module host discovery (when `module_host` is None; only for .NET modules)

1. `FACEBRIDGE_MODULE_HOST` env var; 2. `<exe dir>/module-host/FaceBridge.ModuleHost.exe`;
3. the host bundled into the binary, when it was built with `FACEBRIDGE_BUNDLED_HOST_ZIP=<module-host-<rid>.zip>`:
extracted once to `<data_dir>/bundled-module-host/<hash of the zip>/module-host/` (staged, then renamed into place);
4. dev fallback: `<workspace>/dotnet/artifacts/module-host/FaceBridge.ModuleHost.exe` (what `dotnet/publish.ps1` produces).

### Claim arbitration

Enabled modules start in priority order, one at a time: spawn, wait for `Loaded`, send `Init` with whatever eye /
expression is still unclaimed (skip a module whose `supports` intersects nothing unclaimed: state `Unclaimed`), wait
for `InitResult` (timeout 30 s → kill, `Failed`), record its claims, then the next. A module that claims nothing is
torn down (`Unclaimed`). When a claiming module exits or crashes, its claim is released and its last frame data is
dropped (zeroed); claims are not reassigned until `restart_modules` (hot-swap is future work).

#### Startup live-data check

Some modules claim in `Init` without any headset sending. To stop such a module from blocking a later module whose
headset is sending, the start sequence verifies live data **while the startup lock is held** (so the next module is
not offered anything before the check ended):

1. After `InitResult` granted claims, and if the module's `ModuleOptions.require_data` is true, the engine collects
   the module's `Frame`s for up to `data_timeout_secs` (clamped to 1..=120, default 10). A part is *live* once two
   frames received after `Init` differ in any field of that part. Eye part: the eye fields and the eye-region shapes
   `EyeSquint*` / `EyeWide*` (what the merger takes from the eye owner). Expression part: all other shapes. Head data
   does not count. The wait ends as soon as every checked part is live (no fixed delay for good modules). Frames that
   arrive during the check are stored but the merged output is only published once the check is over.
2. Only parts that some later module could take over are checked: a later *enabled* module in the priority order that
   is not running yet and whose `supports` covers the part (`supports` unknown because the module never loaded counts
   as a candidate). Parts without such a candidate are kept without waiting (a lone module keeps working when the user
   turns the headset on later), logged as "No later module could take over ...". A module started alone (install /
   enable) has no later module, so it is never checked.
3. On timeout, every checked part that is still not live is released (`Merger::release_parts`; its frames are ignored
   from then on) and logged: `No tracking data within N s; eye tracking was offered to the next module`. If the module
   keeps a part, it stays `Running` with the reduced `claims`. If it keeps none, `last_error` gets that text, the module
   is torn down like one that claimed nothing and its state is `NoData` (serialized `nodata`). The released parts are
   offered to the next modules as usual.
4. The check is startup-only: nothing is ever revoked later because data stops (users put the headset down for
   minutes), and there is no periodic retry. `restart_modules` runs the whole sequence again. `require_data = false`
   skips the check (the claim is kept unconditionally, logged as "Live-data check is off for this module").

Merged frame: eye fields from the eye claimer's latest frame; shapes from the expression claimer's, except the
eye-region shapes `EyeSquintRight/Left` and `EyeWideRight/Left` (indices 0..3), which come from the eye claimer
when there is one (otherwise from the expression claimer); head from whichever frame has bit2 set (expression
claimer first). Each `Frame` → update merged state → mutate (`spec/mutation.md`) → `OscOutput::submit`.

### Notifications

Two kinds. `Notify` is a notice: informational, never stored, announced once as `EngineEvent::Notice` (the UI pops
it up and lets it fade). `NotifyMessage` is a message: stored in `EngineSnapshot::notifications` per module, keyed
(`key`, or the message text when key is empty), same key replaces, at most 20 per module (oldest dropped).
`NotifyClear` removes messages. Dismissed by the user via `dismiss_notification`. All of a module's messages are
removed when it stops or restarts.

## fb-mutate and the mutation API

Calibration, unified correctors, parameter adjustment and data filtering of every merged frame before OSC. The
algorithms, JSON shapes and UI are specified in `spec/mutation.md`; this section lists the code contract.

```rust
// fb-mutate (pure: no I/O, threads or clocks), re-exported by fb-core as `fb_core::fb_mutate`
pub struct Mutator { .. }
impl Mutator {
    pub fn new(config: MutationConfig, calibration: CalibrationData) -> Self;
    pub fn config(&self) -> &MutationConfig;
    pub fn set_config(&mut self, config: MutationConfig) -> MutationConfig;          // sanitizes, returns the result
    pub fn calibration(&self) -> &CalibrationData;                                    // one entry per shape
    pub fn reset_calibration(&mut self, shape: Option<fb_tracking::Expression>);      // None = all shapes
    pub fn apply(&mut self, frame: &mut TrackingFrame, now_s: f64);
    pub fn take_calibration_dirty(&mut self) -> bool;
}
pub struct MutationConfig { pub enabled: bool, pub calibration: CalibrationConfig, pub correctors: CorrectorsConfig,
    pub adjustment: AdjustmentConfig, pub filter: FilterConfig }  // Serialize + Deserialize + Clone + Debug + PartialEq + Default; camelCase
impl MutationConfig { pub fn sanitize(&mut self); pub fn sanitized(self) -> Self; }
pub struct CalibrationConfig { pub enabled: bool, pub blend: f32, pub continuous: bool }
pub struct CorrectorsConfig { pub enabled: bool, pub mouth_closed_fix: bool, pub lip_suck_fix: bool, pub eye_lid_blend: f32, pub eye_look_symmetrize: bool }
pub struct AdjustmentConfig { pub enabled: bool, pub ranges: BTreeMap<AdjustGroup, Range> }   // missing group = 0..1
impl AdjustmentConfig { pub fn range(&self, group: AdjustGroup) -> Range; }
pub struct Range { pub min: f32, pub max: f32 }
pub struct FilterConfig { pub enabled: bool, pub eye_gaze: FilterChannel, pub eye_openness: FilterChannel, pub pupil: FilterChannel,
    pub expressions: FilterChannel, pub head: FilterChannel }
pub struct FilterChannel { pub enabled: bool, pub min_cutoff: f32, pub beta: f32, pub derivative_cutoff: f32 }
pub struct CalibrationData { pub shapes: Vec<ShapeCalibration> }                      // + get(Expression), mean_progress()
pub struct ShapeCalibration { pub name: String, pub peak: f32, pub progress: f32 }
pub enum AdjustGroup { .. }   // ALL, name() (camelCase JSON key), label() (UI text), shapes(), is_head(), from_name()

// fb-core
impl Engine {
    pub fn latest_frame(&self) -> (TrackingFrame, fb_osc::Claims);       // the OUTPUT: merged frame after mutation, what OSC sends
    pub fn latest_raw_frame(&self) -> (TrackingFrame, fb_osc::Claims);   // the merged frame before mutation
    pub fn mutation_config(&self) -> MutationConfig;
    pub fn set_mutation_config(&self, cfg: MutationConfig) -> anyhow::Result<MutationConfig>;  // sanitize, apply live, persist config.json, emit MutationChanged; offline: store only
    pub fn calibration(&self) -> CalibrationData;
    pub fn reset_calibration(&self, shape: Option<&str>) -> anyhow::Result<()>;  // Expression name; unknown = error; persists immediately, emits MutationChanged
}
```

`EngineConfig.mutation` (`config.json` → `"mutation"`) holds the settings. `EngineEvent::MutationChanged` follows a
config change or a calibration reset. Learned calibration lives in `<data_dir>/calibration.json`
(`{ "shapes": [{ "name", "peak", "progress" }] }`): loaded at start (missing or corrupt = empty, corrupt is logged),
written by a saver thread when it changed (at most every 10 s), and on reset and `shutdown`; never under the merger
lock. `Inner::submit` applies the mutator under the merger lock (lock order merger → mutator).

CLI: `facebridge-cli mutation show | set <json>` (deep-merges a partial object, prints the sanitized result) and
`facebridge-cli calibration show | reset [Shape]`; both work offline.
