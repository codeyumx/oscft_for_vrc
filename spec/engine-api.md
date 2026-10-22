# Rust crate contracts

Crate graph (arrows = depends on):

```
fb-tracking  <-  fb-protocol  <-  fb-core  ->  fb-osc  ->  fb-tracking
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
    Notify { level: LogLevel, key: String, message: String }, NotifyClear { key: String }, Status { state: i8 },
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
}
pub struct ModuleOptions {        // FaceBridge's options for every module, ModuleEntry.options (camelCase in JSON)
    pub allow_notifications: bool,      // default true; false = Notify only goes to the log (category "notification")
}
pub fn default_data_dir() -> PathBuf;   // %APPDATA%\FaceBridge (dirs::config_dir()/FaceBridge elsewhere)

pub struct Engine { .. }           // shared as Arc<Engine>
impl Engine {
    pub fn start(data_dir: PathBuf) -> anyhow::Result<Arc<Engine>>;   // load config, start OSC, start enabled modules
    pub fn subscribe(&self) -> crossbeam_channel::Receiver<EngineEvent>;
    pub fn snapshot(&self) -> EngineSnapshot;
    pub fn latest_frame(&self) -> (TrackingFrame, fb_osc::Claims);
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

pub enum EngineEvent { ModulesChanged, NotificationsChanged, Log(LogEntry), AvatarChanged, OscStatusChanged }
pub struct EngineSnapshot { pub modules: Vec<ModuleView>, pub notifications: Vec<NotificationView>,
    pub avatar: Option<fb_osc::AvatarInfo>, pub osc: fb_osc::OscStatus, pub osc_config: fb_osc::OscConfig,
    pub logs: Vec<LogEntry> /* ring buffer, newest last, max 1000 */ }
pub struct ModuleView { pub id: String, pub name: String, pub version: Option<String>, pub author: Option<String>,
    pub description: Option<String>, pub usage: Option<String>, pub page_url: Option<String>,
    pub source_repo: Option<String> /* GitHub "owner/repo" from module.json DownloadUrl, else ModulePageUrl */,
    pub options: ModuleOptions, pub enabled: bool, pub state: ModuleRunState, pub supports: Claims, pub claims: Claims,
    pub status: Option<i8>, pub last_error: Option<String>, pub has_settings: bool,
    pub settings_rev: u64 /* changes when the setting values change (user, module, redeclaration) */, pub image_count: usize }
pub enum ModuleRunState { Stopped, Starting, Initializing, Running, Unclaimed, Failed, Exited }   // serialized lowercase
pub struct NotificationView { pub id: u64, pub module_id: String, pub module_name: String, pub level: String /* "information"|"warning"|"error"|... */,
    pub key: String, pub message: String, pub time_ms: u64 }
pub struct LogEntry { pub time_ms: u64, pub module_id: Option<String>, pub level: String, pub category: String, pub message: String }
```

### Module storage

```
<data_dir>/config.json
<data_dir>/modules/<id>/            module files as shipped + module.json (VRCFT TrackingModuleMetadata format:
                                    ModuleId, ModuleName, Version, AuthorName, ModuleDescription, UsageInstructions,
                                    ModulePageUrl, DownloadUrl, DllFileName, ...); id = ModuleId GUID, or a new UUID for local zips
<data_dir>/module-data/<id>/        writable data dir passed as --data-dir; settings.json = current values (without `text` fields)
```

Zip install: extract to a temp dir; DLL = `module.json` `DllFileName` when present, otherwise run
`FaceBridge.ModuleHost --probe <dir>` (stdout JSON `{"modules":[{"dll":"X.dll","type":"Ns.Type"}]}`) and take
the single hit (error if none or several). Zip-slip safe.

### Module host discovery (when `module_host` is None)

1. `FACEBRIDGE_MODULE_HOST` env var; 2. `<exe dir>/module-host/FaceBridge.ModuleHost.exe`;
3. dev fallback: `<workspace>/dotnet/artifacts/module-host/FaceBridge.ModuleHost.exe` (what `dotnet/publish.ps1` produces).

### Claim arbitration

Enabled modules start in priority order, one at a time: spawn, wait for `Loaded`, send `Init` with whatever eye /
expression is still unclaimed (skip a module whose `supports` intersects nothing unclaimed: state `Unclaimed`), wait
for `InitResult` (timeout 30 s → kill, `Failed`), record its claims, then the next. A module that claims nothing is
torn down (`Unclaimed`). When a claiming module exits or crashes, its claim is released and its last frame data is
dropped (zeroed); claims are not reassigned until `restart_modules` (hot-swap is future work).

Merged frame: eye fields from the eye claimer's latest frame; shapes from the expression claimer's, except the
eye-region shapes `EyeSquintRight/Left` and `EyeWideRight/Left` (indices 0..3), which come from the eye claimer
when there is one (otherwise from the expression claimer); head from whichever frame has bit2 set (expression
claimer first). Each `Frame` → update merged state → `OscOutput::submit`.

### Notifications

Per module, keyed (`Notify.key`, or the message text when key is empty). Same key replaces. `NotifyClear` removes.
Dismissed by the user via `dismiss_notification`. All of a module's notifications are removed when it stops or
restarts.
