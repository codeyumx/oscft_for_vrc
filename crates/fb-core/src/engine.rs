//! The engine: shared state, public API, installs, settings, lifecycle.

use crate::config::{EngineConfig, ModuleEntry, ModuleOptions, Paths, load_config, save_config, write_json_atomic};
use crate::host::{discover_module_host, probe_modules};
use crate::merge::{Merger, RunKey};
use crate::notifications::Notifications;
use crate::run::{self, RunCtl};
use crate::settings::Schema;
use crate::store::{self, ModuleMeta};
use crate::types::{
    EngineEvent, EngineSnapshot, LogEntry, ModuleRunState, ModuleSettingsView, ModuleView,
};
use anyhow::{Context, Result, anyhow, bail};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use fb_mutate::{CalibrationData, MutationConfig, Mutator};
use fb_osc::{Claims, OscConfig, OscEvent, OscOutput, OscStatus};
use fb_tracking::{Expression, TrackingFrame};
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub type ModuleId = String;

/// Log ring buffer size.
pub const LOG_CAPACITY: usize = 1000;

/// Teardown grace period before a module host is killed.
pub(crate) const TEARDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Subscriber list; every event goes to every live subscriber.
#[derive(Default)]
pub(crate) struct Events {
    subs: Mutex<Vec<Sender<EngineEvent>>>,
}

impl Events {
    pub fn emit(&self, event: EngineEvent) {
        lock(&self.subs).retain(|tx| tx.send(event.clone()).is_ok());
    }

    pub fn subscribe(&self) -> Receiver<EngineEvent> {
        let (tx, rx) = crossbeam_channel::unbounded();
        lock(&self.subs).push(tx);
        rx
    }
}

/// Runtime view of one installed module.
pub(crate) struct ModuleRt {
    pub meta: ModuleMeta,
    pub state: ModuleRunState,
    pub supports: Claims,
    /// `supports` was reported by the module host at least once (it is kept across restarts), so it can be
    /// relied on to decide whether the module could take over a part.
    pub supports_known: bool,
    pub claims: Claims,
    pub status: Option<i8>,
    pub last_error: Option<String>,
    pub schema: Option<Schema>,
    pub values: Map<String, Value>,
    /// Whether `settings.json` exists (values are only pushed to the child before `Init` when it does).
    pub stored: bool,
    /// Bumped whenever `values` change (user edit, module write, redeclaration), so the UI can re-read them.
    pub settings_rev: u64,
    pub images: Vec<Vec<u8>>,
    pub init_name: Option<String>,
    pub run: Option<Arc<RunCtl>>,
}

impl ModuleRt {
    pub fn load(paths: &Paths, id: &str) -> Self {
        let mut rt = Self {
            meta: ModuleMeta::default(),
            state: ModuleRunState::Stopped,
            supports: Claims::default(),
            supports_known: false,
            claims: Claims::default(),
            status: None,
            last_error: None,
            schema: None,
            values: Map::new(),
            stored: false,
            settings_rev: 0,
            images: Vec::new(),
            init_name: None,
            run: None,
        };
        let dir = paths.module_dir(id);
        match ModuleMeta::read(&dir) {
            Ok(meta) => {
                match (meta.executable.as_deref(), meta.dll_file_name.as_deref()) {
                    (Some(exe), _) if store::native_executable(&dir, exe).is_file() => {}
                    (Some(exe), _) => rt.fail_load(format!("module executable {exe} is missing")),
                    (None, Some(dll)) if dir.join(dll).is_file() => {}
                    (None, Some(dll)) => rt.fail_load(format!("module DLL {dll} is missing")),
                    (None, None) => rt.fail_load("module.json has neither Executable nor DllFileName".into()),
                }
                rt.meta = meta;
            }
            Err(e) => rt.fail_load(format!("{e:#}")),
        }
        if let Ok(text) = std::fs::read_to_string(paths.schema_cache_path(id)) {
            rt.schema = Schema::parse(&text).ok();
        }
        if let Ok(text) = std::fs::read_to_string(paths.settings_path(id)) {
            if let Ok(Value::Object(stored)) = serde_json::from_str(&text) {
                rt.stored = true;
                rt.values = stored;
            }
        }
        if let Some(schema) = &rt.schema {
            rt.values = schema.normalize(&rt.values);
        }
        rt
    }

    fn fail_load(&mut self, error: String) {
        self.state = ModuleRunState::Failed;
        self.last_error = Some(error);
    }

    pub fn display_name(&self, id: &str) -> String {
        self.meta.module_name.clone().or_else(|| self.init_name.clone()).unwrap_or_else(|| id.to_owned())
    }

    /// The values that go to disk and to the module (display text is runtime-only).
    pub fn persisted_values(&self) -> Map<String, Value> {
        match &self.schema {
            Some(schema) => schema.persisted(&self.values),
            None => self.values.clone(),
        }
    }
}

pub(crate) struct State {
    pub config: EngineConfig,
    pub modules: HashMap<String, ModuleRt>,
    pub notifications: Notifications,
    pub logs: VecDeque<LogEntry>,
    /// Bumped by restart/shutdown; start sequences from an older generation abandon their work.
    pub generation: u64,
}

pub(crate) struct Inner {
    pub paths: Paths,
    pub events: Arc<Events>,
    pub state: Mutex<State>,
    pub merger: Mutex<Merger>,
    /// Calibration, correctors, adjustment and filter. Lock order: merger, then mutator.
    pub mutator: Mutex<Mutator>,
    /// What was last sent to OSC (the mutated frame).
    pub last_output: Mutex<(TrackingFrame, Claims)>,
    /// Monotonic clock for the mutator.
    clock: Instant,
    pub calibration_saver: Mutex<CalibrationSaver>,
    saver_thread: Mutex<Option<(Sender<()>, JoinHandle<()>)>>,
    pub osc: RwLock<Option<OscOutput>>,
    /// Held while one module goes through spawn → Init so claim offers never race.
    pub startup: Mutex<()>,
    pub shutting_down: AtomicBool,
    next_run: AtomicU64,
    /// Management mode: nothing is started (see [`Engine::open_offline`]).
    pub offline: bool,
}

impl Inner {
    pub fn next_run_key(&self) -> RunKey {
        self.next_run.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn emit(&self, event: EngineEvent) {
        self.events.emit(event);
    }

    pub fn log(&self, module_id: Option<&str>, level: &str, category: &str, message: impl Into<String>) {
        let entry = LogEntry {
            time_ms: now_ms(),
            module_id: module_id.map(str::to_owned),
            level: level.to_owned(),
            category: category.to_owned(),
            message: message.into(),
        };
        {
            let mut st = lock(&self.state);
            if st.logs.len() >= LOG_CAPACITY {
                st.logs.pop_front();
            }
            st.logs.push_back(entry.clone());
        }
        self.emit(EngineEvent::Log(entry));
    }

    /// Runs the merged frame through the mutator and submits the result to OSC. Callers hold the merger lock so
    /// submissions stay ordered (lock order: merger, then mutator).
    pub fn submit(&self, frame: &TrackingFrame, claims: Claims) {
        let mut out = *frame;
        lock(&self.mutator).apply(&mut out, self.clock.elapsed().as_secs_f64());
        *lock(&self.last_output) = (out, claims);
        if let Some(osc) = self.osc.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
            osc.submit(&out, claims);
        }
    }

    /// Runs the current merged frame through the (possibly changed) mutator again so the output follows a
    /// settings change even while no module sends frames.
    pub fn resubmit(&self) {
        if self.offline || self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let merger = lock(&self.merger);
        let (frame, claims) = merger.merged();
        self.submit(&frame, claims);
    }

    /// Drops all claims and frame data, including the output frame.
    pub fn clear_merger(&self) {
        let mut merger = lock(&self.merger);
        merger.clear();
        *lock(&self.last_output) = (TrackingFrame::default(), Claims::default());
    }

    /// Writes the calibration data when it changed since the last write: at most every
    /// [`CALIBRATION_SAVE_INTERVAL`], or right away with `force`. The file is written without holding the merger
    /// or mutator lock.
    pub fn flush_calibration(&self, force: bool) -> Result<()> {
        let mut saver = lock(&self.calibration_saver);
        let data = {
            let mut mutator = lock(&self.mutator);
            if mutator.take_calibration_dirty() {
                saver.unsaved = true;
            }
            if !saver.unsaved || (!force && saver.last_save.elapsed() < CALIBRATION_SAVE_INTERVAL) {
                return Ok(());
            }
            mutator.calibration().clone()
        };
        saver.last_save = Instant::now();
        write_json_atomic(&self.paths.calibration_path(), &data).context("saving calibration")?;
        saver.unsaved = false;
        Ok(())
    }

    /// Stops the calibration saver thread (idempotent).
    pub fn stop_calibration_saver(&self) {
        if let Some((stop, handle)) = lock(&self.saver_thread).take() {
            drop(stop);
            let _ = handle.join();
        }
    }

    /// Shows a module notification, unless the user turned notifications off for that module (then it only goes
    /// to the log). A notice (`persistent == false`) is only announced as [`EngineEvent::Notice`]; a message is
    /// stored in the snapshot until dismissed or cleared.
    pub fn notify(&self, id: &str, level: &str, key: &str, message: &str, persistent: bool) {
        let event = {
            let mut st = lock(&self.state);
            let allowed = st.config.modules.iter().find(|m| m.id == id).is_none_or(|m| m.options.allow_notifications);
            if !allowed {
                drop(st);
                self.log(Some(id), level, "notification", format!("(notifications off for this module) {message}"));
                return;
            }
            let name = st.modules.get(id).map_or_else(|| id.to_owned(), |m| m.display_name(id));
            if persistent {
                st.notifications.message(id, &name, level, key, message, now_ms());
                EngineEvent::NotificationsChanged
            } else {
                EngineEvent::Notice(st.notifications.notice(id, &name, level, key, message, now_ms()))
            }
        };
        self.emit(event);
    }

    pub fn clear_notifications(&self, id: &str, key: &str) {
        let changed = lock(&self.state).notifications.clear(id, key);
        if changed {
            self.emit(EngineEvent::NotificationsChanged);
        }
    }

    /// Runs `f` on the module and announces the change.
    pub fn update_module<R>(&self, id: &str, f: impl FnOnce(&mut ModuleRt) -> R) -> Option<R> {
        let r = lock(&self.state).modules.get_mut(id).map(f);
        if r.is_some() {
            self.emit(EngineEvent::ModulesChanged);
        }
        r
    }

    fn start_osc(&self, config: OscConfig) -> std::io::Result<OscOutput> {
        let events = Arc::clone(&self.events);
        OscOutput::start(config, move |e| match e {
            OscEvent::AvatarChanged(_) => events.emit(EngineEvent::AvatarChanged),
            OscEvent::StatusChanged(_) => events.emit(EngineEvent::OscStatusChanged),
        })
    }
}

/// Calibration is written at most this often while it keeps changing.
const CALIBRATION_SAVE_INTERVAL: Duration = Duration::from_secs(10);
/// How often the saver thread looks at the calibration state.
const CALIBRATION_SAVE_POLL: Duration = Duration::from_secs(1);

/// Persistence bookkeeping of the calibration data.
pub(crate) struct CalibrationSaver {
    /// Changed since the last successful write.
    unsaved: bool,
    last_save: Instant,
}

/// Missing or corrupt `calibration.json` means empty data; a corrupt file also returns a message to log.
fn load_calibration(path: &Path) -> (CalibrationData, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(data) => (data, None),
            Err(e) => (CalibrationData::default(), Some(format!("Ignoring corrupt {}: {e}", path.display()))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (CalibrationData::default(), None),
        Err(e) => (CalibrationData::default(), Some(format!("Could not read {}: {e}", path.display()))),
    }
}

/// Starts the thread that writes changed calibration data to disk; [`Inner::stop_calibration_saver`] ends it.
fn spawn_calibration_saver(inner: &Arc<Inner>) {
    let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(0);
    let worker = Arc::clone(inner);
    let spawned = std::thread::Builder::new().name("fb-calibration-saver".into()).spawn(move || {
        while let Err(RecvTimeoutError::Timeout) = stop_rx.recv_timeout(CALIBRATION_SAVE_POLL) {
            if let Err(e) = worker.flush_calibration(false) {
                worker.log(None, "error", "engine", format!("{e:#}"));
            }
        }
    });
    match spawned {
        Ok(handle) => *lock(&inner.saver_thread) = Some((stop_tx, handle)),
        Err(e) => inner.log(None, "error", "engine", format!("Calibration will not be saved while running: {e}")),
    }
}

/// Module ids become directory names, so they must be canonical lower-case hyphenated UUIDs.
fn valid_module_id(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|u| u.hyphenated().to_string() == id)
}

fn vrcft_custom_libs_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("VRCFaceTracking").join("CustomLibs"))
}

pub struct Engine {
    inner: Arc<Inner>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Engine {
    /// Loads the config under `data_dir`, starts OSC output and begins starting the enabled modules in priority
    /// order on a background thread (the call does not wait for modules; watch [`EngineEvent::ModulesChanged`]).
    pub fn start(data_dir: PathBuf) -> Result<Arc<Engine>> {
        Self::launch(data_dir, false)
    }

    /// Management mode for installs, settings and listing: loads the config and modules but starts neither OSC
    /// output nor module processes (enabling, installing and restarting only change what is stored).
    pub fn open_offline(data_dir: PathBuf) -> Result<Arc<Engine>> {
        Self::launch(data_dir, true)
    }

    fn launch(data_dir: PathBuf, offline: bool) -> Result<Arc<Engine>> {
        let data_dir = std::path::absolute(&data_dir).unwrap_or(data_dir);
        std::fs::create_dir_all(&data_dir).with_context(|| format!("creating {}", data_dir.display()))?;
        let mut config = load_config(&data_dir)?;
        config.mutation.sanitize();
        let paths = Paths::new(data_dir);
        let (calibration, calibration_warning) = load_calibration(&paths.calibration_path());

        let mut seen = std::collections::HashSet::new();
        config.modules.retain(|m| valid_module_id(&m.id) && seen.insert(m.id.clone()));
        let modules = config.modules.iter().map(|m| (m.id.clone(), ModuleRt::load(&paths, &m.id))).collect();

        let inner = Arc::new(Inner {
            paths,
            events: Arc::new(Events::default()),
            state: Mutex::new(State {
                config: config.clone(),
                modules,
                notifications: Notifications::default(),
                logs: VecDeque::new(),
                generation: 0,
            }),
            merger: Mutex::new(Merger::default()),
            mutator: Mutex::new(Mutator::new(config.mutation.clone(), calibration)),
            last_output: Mutex::new((TrackingFrame::default(), Claims::default())),
            clock: Instant::now(),
            calibration_saver: Mutex::new(CalibrationSaver { unsaved: false, last_save: Instant::now() }),
            saver_thread: Mutex::new(None),
            osc: RwLock::new(None),
            startup: Mutex::new(()),
            shutting_down: AtomicBool::new(false),
            next_run: AtomicU64::new(0),
            offline,
        });
        if let Some(warning) = calibration_warning {
            inner.log(None, "warning", "engine", warning);
        }
        if !offline {
            let osc = inner.start_osc(config.osc.clone()).context("starting OSC output")?;
            *inner.osc.write().unwrap_or_else(|e| e.into_inner()) = Some(osc);
            spawn_calibration_saver(&inner);
        }

        let engine = Arc::new(Engine { inner });
        run::spawn_start_sequence(&engine.inner, 0);
        Ok(engine)
    }

    pub fn subscribe(&self) -> Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let (modules, notifications, logs, osc_config) = {
            let st = lock(&self.inner.state);
            let modules = st
                .config
                .modules
                .iter()
                .filter_map(|entry| {
                    let rt = st.modules.get(&entry.id)?;
                    Some(ModuleView {
                        id: entry.id.clone(),
                        name: rt.display_name(&entry.id),
                        version: rt.meta.version.clone(),
                        author: rt.meta.author_name.clone(),
                        description: rt.meta.module_description.clone(),
                        usage: rt.meta.usage_instructions.clone(),
                        page_url: rt.meta.module_page_url.clone(),
                        source_repo: rt.meta.source_repo(),
                        options: entry.options,
                        enabled: entry.enabled,
                        state: rt.state,
                        supports: rt.supports,
                        claims: rt.claims,
                        status: rt.status,
                        last_error: rt.last_error.clone(),
                        has_settings: rt.schema.is_some(),
                        settings_rev: rt.settings_rev,
                        image_count: rt.images.len(),
                    })
                })
                .collect::<Vec<_>>();
            (
                modules,
                st.notifications.list().to_vec(),
                st.logs.iter().cloned().collect::<Vec<_>>(),
                st.config.osc.clone(),
            )
        };
        let osc_guard = self.inner.osc.read().unwrap_or_else(|e| e.into_inner());
        let (osc, avatar) = match osc_guard.as_ref() {
            Some(o) => (o.status(), o.avatar()),
            None => (
                OscStatus {
                    send_addr: osc_config.send_addr,
                    sending: false,
                    messages_per_second: 0,
                    last_error: Some("OSC output is not running".into()),
                    vrchat_detected: false,
                },
                None,
            ),
        };
        EngineSnapshot { modules, notifications, avatar, osc, osc_config, logs }
    }

    /// The current output frame (what goes to OSC: the merged frame after mutation) and which parts are claimed.
    pub fn latest_frame(&self) -> (TrackingFrame, Claims) {
        *lock(&self.inner.last_output)
    }

    /// The merged frame before calibration, correctors, adjustment and filtering.
    pub fn latest_raw_frame(&self) -> (TrackingFrame, Claims) {
        lock(&self.inner.merger).merged()
    }

    /// Installs a `.zip` or a module folder (replacing an existing install of the same module id). The new module
    /// is enabled and started right away with whatever eye / expression is still unclaimed.
    pub fn install_module(&self, path: &Path) -> Result<ModuleId> {
        let inner = &self.inner;
        let (host_cfg, dotnet_root) = {
            let st = lock(&inner.state);
            (st.config.module_host.clone(), st.config.dotnet_root.clone())
        };
        let probe = |dir: &Path| {
            let host = discover_module_host(host_cfg.as_deref(), &inner.paths.data_dir)?;
            probe_modules(&host, dotnet_root.as_deref(), dir)
        };
        let staged = store::stage(&inner.paths, path, &probe)?;
        let id = staged.id.clone();
        let name = staged.meta.module_name.clone().unwrap_or_else(|| id.clone());

        let previous = lock(&inner.state).modules.get(&id).and_then(|m| m.run.clone());
        if let Some(ctl) = previous {
            run::stop_run(&ctl);
        }
        store::commit(&inner.paths, staged)?;

        let (enabled, generation) = {
            let mut st = lock(&inner.state);
            st.modules.insert(id.clone(), ModuleRt::load(&inner.paths, &id));
            if !st.config.modules.iter().any(|m| m.id == id) {
                st.config.modules.push(ModuleEntry::new(id.clone()));
            }
            let enabled = st.config.modules.iter().find(|m| m.id == id).is_some_and(|m| m.enabled);
            save_config(&inner.paths.data_dir, &st.config)?;
            (enabled, st.generation)
        };
        inner.log(None, "information", "engine", format!("Installed module {name} ({id})"));
        inner.emit(EngineEvent::ModulesChanged);
        if enabled {
            run::spawn_hot_start(inner, &id, generation);
        }
        Ok(id)
    }

    /// Copies every module folder under `%APPDATA%\VRCFaceTracking\CustomLibs` into FaceBridge's storage. The VRCFT
    /// folder is only read.
    pub fn import_vrcft_modules(&self) -> Result<Vec<ModuleId>> {
        let dir = vrcft_custom_libs_dir().context("cannot locate the user config directory")?;
        self.import_modules_from(&dir)
    }

    pub(crate) fn import_modules_from(&self, dir: &Path) -> Result<Vec<ModuleId>> {
        if !dir.is_dir() {
            bail!("VRCFaceTracking CustomLibs folder not found: {}", dir.display());
        }
        let mut found = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.path().join("module.json").is_file())
            .map(|e| e.path())
            .collect::<Vec<_>>();
        found.sort();
        let mut ids = Vec::new();
        let mut errors = Vec::new();
        for path in found {
            match self.install_module(&path) {
                Ok(id) => ids.push(id),
                Err(e) => {
                    let msg = format!("Could not import {}: {e:#}", path.display());
                    self.inner.log(None, "warning", "engine", msg.clone());
                    errors.push(msg);
                }
            }
        }
        if ids.is_empty() && !errors.is_empty() {
            bail!("{}", errors.join("; "));
        }
        Ok(ids)
    }

    pub fn uninstall_module(&self, id: &str) -> Result<()> {
        let inner = &self.inner;
        let ctl = {
            let st = lock(&inner.state);
            if !st.modules.contains_key(id) {
                bail!("no such module: {id}");
            }
            st.modules[id].run.clone()
        };
        if let Some(ctl) = ctl {
            run::stop_run(&ctl);
        }
        let name = {
            let mut st = lock(&inner.state);
            let name = st.modules.get(id).map_or_else(|| id.to_owned(), |m| m.display_name(id));
            st.modules.remove(id);
            st.config.modules.retain(|m| m.id != id);
            st.notifications.clear_module(id);
            save_config(&inner.paths.data_dir, &st.config)?;
            name
        };
        inner.emit(EngineEvent::NotificationsChanged);
        inner.emit(EngineEvent::ModulesChanged);
        store::remove_module_files(&inner.paths, id)?;
        inner.log(None, "information", "engine", format!("Removed module {name}"));
        Ok(())
    }

    /// Enabling starts the module now with whatever is unclaimed; disabling stops it (its claim is released, not
    /// reassigned).
    pub fn set_module_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let inner = &self.inner;
        let (ctl, generation) = {
            let mut st = lock(&inner.state);
            let Some(entry) = st.config.modules.iter_mut().find(|m| m.id == id) else {
                bail!("no such module: {id}");
            };
            entry.enabled = enabled;
            save_config(&inner.paths.data_dir, &st.config)?;
            (st.modules.get(id).and_then(|m| m.run.clone()), st.generation)
        };
        inner.emit(EngineEvent::ModulesChanged);
        if enabled {
            run::spawn_hot_start(inner, id, generation);
        } else if let Some(ctl) = ctl {
            run::stop_run(&ctl);
        }
        Ok(())
    }

    /// Sets the module priority order (first = highest). Takes effect on the next [`Engine::restart_modules`].
    /// Installed modules missing from `ids` keep their relative order after the listed ones.
    pub fn set_module_order(&self, ids: &[String]) -> Result<()> {
        let mut st = lock(&self.inner.state);
        let mut remaining = std::mem::take(&mut st.config.modules);
        let mut ordered = Vec::with_capacity(remaining.len());
        for id in ids {
            match remaining.iter().position(|m| &m.id == id) {
                Some(i) => ordered.push(remaining.remove(i)),
                None if ordered.iter().any(|m| &m.id == id) => {}
                None => {
                    remaining.splice(0..0, ordered);
                    st.config.modules = remaining;
                    bail!("no such module: {id}");
                }
            }
        }
        ordered.append(&mut remaining);
        st.config.modules = ordered;
        save_config(&self.inner.paths.data_dir, &st.config)?;
        drop(st);
        self.inner.emit(EngineEvent::ModulesChanged);
        Ok(())
    }

    /// Stops every module and starts the enabled ones again in priority order, re-running claim arbitration.
    /// Returns once the old processes are gone; startup continues in the background.
    pub fn restart_modules(&self) -> Result<()> {
        let inner = &self.inner;
        let (generation, ctls) = run::bump_generation(inner);
        run::stop_many(&ctls);
        inner.clear_merger();
        inner.emit(EngineEvent::ModulesChanged);
        if !inner.shutting_down.load(Ordering::SeqCst) {
            run::spawn_start_sequence(inner, generation);
        }
        Ok(())
    }

    pub fn module_settings(&self, id: &str) -> Option<ModuleSettingsView> {
        let st = lock(&self.inner.state);
        let rt = st.modules.get(id)?;
        let schema = rt.schema.as_ref()?;
        Some(ModuleSettingsView { schema: schema.raw.clone(), values: Value::Object(rt.values.clone()) })
    }

    /// Applies a partial values object, validated against the module's schema (unknown keys and wrong types
    /// dropped, numbers clamped), persists it and pushes it to the running module.
    pub fn set_module_settings(&self, id: &str, values: Value) -> Result<()> {
        let Value::Object(patch) = values else {
            bail!("settings must be a JSON object");
        };
        apply_settings(&self.inner, id, &patch, true)
    }

    /// Changes the options FaceBridge offers for every module and persists them. Turning notifications off
    /// removes the module's current notifications.
    pub fn set_module_options(&self, id: &str, options: ModuleOptions) -> Result<()> {
        let inner = &self.inner;
        let cleared = {
            let mut st = lock(&inner.state);
            let entry = st.config.modules.iter_mut().find(|m| m.id == id).with_context(|| format!("no such module: {id}"))?;
            entry.options = ModuleOptions {
                data_timeout_secs: options
                    .data_timeout_secs
                    .clamp(ModuleOptions::MIN_DATA_TIMEOUT_SECS, ModuleOptions::MAX_DATA_TIMEOUT_SECS),
                ..options
            };
            save_config(&inner.paths.data_dir, &st.config)?;
            !options.allow_notifications && st.notifications.clear_module(id)
        };
        if cleared {
            inner.emit(EngineEvent::NotificationsChanged);
        }
        inner.emit(EngineEvent::ModulesChanged);
        Ok(())
    }

    /// Tells the running module that the user pressed its settings button `key`.
    pub fn press_module_button(&self, id: &str, key: &str) -> Result<()> {
        let ctl = {
            let st = lock(&self.inner.state);
            let rt = st.modules.get(id).with_context(|| format!("no such module: {id}"))?;
            let name = rt.display_name(id);
            if !rt.schema.as_ref().is_some_and(|s| s.has_button(key)) {
                bail!("{name} has no button \"{key}\"");
            }
            rt.run
                .clone()
                .filter(|c| c.init_sent.load(Ordering::SeqCst))
                .with_context(|| format!("{name} is not running"))?
        };
        ctl.send(&fb_protocol::HostMessage::SettingsAction { key: key.to_owned() })
            .map_err(|e| anyhow!("could not reach the module: {e}"))
    }

    pub fn module_image(&self, id: &str, index: usize) -> Option<Vec<u8>> {
        lock(&self.inner.state).modules.get(id)?.images.get(index).cloned()
    }

    pub fn dismiss_notification(&self, id: u64) {
        if lock(&self.inner.state).notifications.dismiss(id) {
            self.inner.emit(EngineEvent::NotificationsChanged);
        }
    }

    /// Restarts OSC output with a new configuration and persists it. On failure the previous configuration keeps
    /// running.
    pub fn set_osc_config(&self, osc: OscConfig) -> Result<()> {
        let inner = &self.inner;
        if inner.shutting_down.load(Ordering::SeqCst) {
            bail!("the engine is shut down");
        }
        if inner.offline {
            let mut st = lock(&inner.state);
            st.config.osc = osc;
            return save_config(&inner.paths.data_dir, &st.config);
        }
        let previous = lock(&inner.state).config.osc.clone();
        {
            let mut slot = inner.osc.write().unwrap_or_else(|e| e.into_inner());
            // Release the old sockets before binding the new ones.
            drop(slot.take());
            match inner.start_osc(osc.clone()) {
                Ok(new) => *slot = Some(new),
                Err(e) => {
                    *slot = inner.start_osc(previous).ok();
                    drop(slot);
                    inner.emit(EngineEvent::OscStatusChanged);
                    return Err(e).context("starting OSC output with the new settings");
                }
            }
        }
        {
            let mut st = lock(&inner.state);
            st.config.osc = osc;
            save_config(&inner.paths.data_dir, &st.config)?;
        }
        {
            let merger = lock(&inner.merger);
            let (frame, claims) = merger.merged();
            inner.submit(&frame, claims);
        }
        inner.emit(EngineEvent::OscStatusChanged);
        inner.emit(EngineEvent::AvatarChanged);
        Ok(())
    }

    /// The current mutation settings (sanitized).
    pub fn mutation_config(&self) -> MutationConfig {
        lock(&self.inner.state).config.mutation.clone()
    }

    /// Sanitizes `cfg`, applies it to the running frames, persists it in `config.json`, announces
    /// [`EngineEvent::MutationChanged`] and returns the sanitized config. In offline mode it is only stored.
    pub fn set_mutation_config(&self, cfg: MutationConfig) -> Result<MutationConfig> {
        let inner = &self.inner;
        if inner.shutting_down.load(Ordering::SeqCst) {
            bail!("the engine is shut down");
        }
        let cfg = cfg.sanitized();
        {
            let mut st = lock(&inner.state);
            let previous = std::mem::replace(&mut st.config.mutation, cfg.clone());
            if let Err(e) = save_config(&inner.paths.data_dir, &st.config) {
                st.config.mutation = previous;
                return Err(e);
            }
            lock(&inner.mutator).set_config(cfg.clone());
        }
        inner.resubmit();
        inner.emit(EngineEvent::MutationChanged);
        Ok(cfg)
    }

    /// The learned calibration of every shape.
    pub fn calibration(&self) -> CalibrationData {
        lock(&self.inner.mutator).calibration().clone()
    }

    /// Forgets the learned calibration of the shape named `shape` (an `Expression` name such as `"JawOpen"`), or
    /// of every shape for `None`, and persists the result. An unknown name is an error.
    pub fn reset_calibration(&self, shape: Option<&str>) -> Result<()> {
        let inner = &self.inner;
        let expression = shape
            .map(|name| Expression::from_name(name).with_context(|| format!("unknown shape: {name}")))
            .transpose()?;
        lock(&inner.mutator).reset_calibration(expression);
        inner.resubmit();
        inner.flush_calibration(true)?;
        inner.emit(EngineEvent::MutationChanged);
        Ok(())
    }

    /// Tears down all modules (Teardown, 5 s, then kill) and stops OSC. Idempotent.
    pub fn shutdown(&self) {
        let inner = &self.inner;
        if inner.shutting_down.swap(true, Ordering::SeqCst) {
            return;
        }
        let (_, ctls) = run::bump_generation(inner);
        run::stop_many(&ctls);
        inner.clear_merger();
        drop(inner.osc.write().unwrap_or_else(|e| e.into_inner()).take());
        inner.stop_calibration_saver();
        if let Err(e) = inner.flush_calibration(true) {
            inner.log(None, "error", "engine", format!("{e:#}"));
        }
    }
}

/// Validates `patch` against the module's schema and stores it; `push` also sends it to the running child.
/// (Used for both user edits and the module's own `SettingsWrite`; the latter must not echo back.)
pub(crate) fn apply_settings(inner: &Inner, id: &str, patch: &Map<String, Value>, push: bool) -> Result<()> {
    let (values, ctl) = {
        let mut st = lock(&inner.state);
        let Some(rt) = st.modules.get_mut(id) else {
            bail!("no such module: {id}");
        };
        let values = match &rt.schema {
            Some(schema) => schema.apply(&rt.values, patch, push),
            None if push => bail!("module {id} declares no settings"),
            None => {
                let mut merged = rt.values.clone();
                merged.extend(patch.iter().map(|(k, v)| (k.clone(), v.clone())));
                merged
            }
        };
        rt.values = values;
        let persisted = rt.persisted_values();
        write_json_atomic(&inner.paths.settings_path(id), &persisted)?;
        rt.stored = true;
        rt.settings_rev += 1;
        (persisted, rt.run.clone())
    };
    if push {
        if let Some(ctl) = ctl {
            if ctl.init_sent.load(Ordering::SeqCst) {
                let json = Value::Object(values).to_string();
                if let Err(e) = ctl.send(&fb_protocol::HostMessage::Settings { json }) {
                    inner.log(Some(id), "warning", "engine", format!("Could not push settings to the module: {e}"));
                }
            }
        }
    }
    inner.emit(EngineEvent::ModulesChanged);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_module(dir: &Path, id: &str, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("module.json"),
            json!({ "ModuleId": id, "ModuleName": name, "DllFileName": format!("{name}.dll") }).to_string(),
        )
        .unwrap();
        std::fs::write(dir.join(format!("{name}.dll")), b"not a real dll").unwrap();
    }

    fn snapshot_dir(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    out.push((e.path().strip_prefix(dir).unwrap().display().to_string(), std::fs::read(e.path()).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn importing_a_vrcft_custom_libs_folder_copies_and_leaves_the_source_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        // A module host that does not exist: imported modules fail to start, which is irrelevant here.
        let cfg = json!({
            "moduleHost": tmp.path().join("missing-host.exe"),
            "osc": { "sendAddr": "127.0.0.1:9", "listenPort": 0, "oscquery": false },
        });
        std::fs::write(data.join("config.json"), cfg.to_string()).unwrap();
        let libs = tmp.path().join("CustomLibs");
        let (a, b) = ("103a993c-383f-4a7a-8cc6-5e8b4bf0d133", "168d44d5-bbb7-4e86-8ae5-732047311d43");
        write_module(&libs.join(a), a, "Alpha");
        write_module(&libs.join(b), b, "Beta");
        std::fs::write(libs.join("loose-archive.tar.gz"), b"ignored").unwrap();
        // A broken folder (module.json names a DLL that is not there) is skipped, not fatal.
        std::fs::create_dir_all(libs.join("broken")).unwrap();
        std::fs::write(libs.join("broken").join("module.json"), r#"{"DllFileName":"Nope.dll"}"#).unwrap();
        let before = snapshot_dir(&libs);

        let engine = Engine::start(data.clone()).unwrap();
        let mut ids = engine.import_modules_from(&libs).unwrap();
        ids.sort();
        assert_eq!(ids, [a, b]);
        assert_eq!(snapshot_dir(&libs), before, "the VRCFT folder must not be modified");
        assert!(data.join("modules").join(a).join("Alpha.dll").is_file());
        assert!(data.join("modules").join(b).join("Beta.dll").is_file());
        assert_eq!(engine.snapshot().modules.len(), 2);
        assert!(engine.snapshot().logs.iter().any(|l| l.level == "warning" && l.message.contains("broken")));
        assert_eq!(load_config(&data).unwrap().modules.len(), 2);
        engine.shutdown();

        assert!(engine.import_modules_from(&tmp.path().join("nope")).is_err());
    }

    #[test]
    fn module_ids_from_a_hand_edited_config_must_be_canonical_uuids() {
        assert!(valid_module_id("103a993c-383f-4a7a-8cc6-5e8b4bf0d133"));
        for bad in ["", "..", "../x", "103A993C-383F-4A7A-8CC6-5E8B4BF0D133", "103a993c383f4a7a8cc65e8b4bf0d133"] {
            assert!(!valid_module_id(bad), "{bad:?}");
        }
    }
}
