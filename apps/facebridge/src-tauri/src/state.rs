//! Engine lifecycle owned by the app: start in the background (so the window always opens), forward engine
//! events to the webview, stream tracking frames while the Live view is open, and shut down cleanly.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle, Thread};
use std::time::Duration;

use fb_core::{Engine, EngineEvent};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::updates::{self, AppSettings, UpdateChecker};
use crate::view::FrameView;

/// Frame push interval for the Live view (~20 Hz).
const FRAME_INTERVAL: Duration = Duration::from_millis(50);
/// How often an event forwarder re-checks whether its engine generation is still current.
const FORWARDER_POLL: Duration = Duration::from_millis(500);
/// How often the update checker re-evaluates which module sources are due (it is also woken on module changes).
const UPDATE_POLL: Duration = Duration::from_secs(60);

pub const EVENT_ENGINE_STATE: &str = "engine-state";
pub const EVENT_ENGINE: &str = "engine-event";
pub const EVENT_FRAME: &str = "frame";
/// Emitted when a module source's latest release changed (the UI re-reads `get_state`).
pub const EVENT_MODULE_UPDATES: &str = "module-updates";

enum Slot {
    Starting,
    Running(Arc<Engine>),
    Failed(String),
    Stopped,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    /// `starting` | `running` | `failed` | `stopped`
    pub phase: &'static str,
    pub error: Option<String>,
}

pub struct Supervisor {
    pub data_dir: PathBuf,
    slot: Mutex<Slot>,
    /// Bumped on every start and shutdown; event forwarders of older generations exit.
    generation: AtomicU64,
    starter: Mutex<Option<JoinHandle<()>>>,
    live: AtomicBool,
    /// Whether the Live view shows the merged input (`true`) or the mutated output.
    live_raw: AtomicBool,
    stopping: AtomicBool,
    live_thread: OnceLock<Thread>,
    pub updates: UpdateChecker,
    update_thread: OnceLock<Thread>,
    /// Mirror of `app-settings.json`; changed only through `update_settings` so a save never drops another setting.
    settings: Mutex<AppSettings>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Supervisor {
    pub fn new(data_dir: PathBuf) -> Arc<Self> {
        let settings = updates::load_settings(&data_dir).unwrap_or_else(|e| {
            eprintln!("facebridge: {e:#}; using default app settings");
            Default::default()
        });
        Arc::new(Self {
            data_dir,
            slot: Mutex::new(Slot::Stopped),
            generation: AtomicU64::new(0),
            starter: Mutex::new(None),
            live: AtomicBool::new(false),
            live_raw: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            live_thread: OnceLock::new(),
            updates: UpdateChecker::new(settings.check_module_updates),
            update_thread: OnceLock::new(),
            settings: Mutex::new(settings),
        })
    }

    pub fn settings(&self) -> AppSettings {
        lock(&self.settings).clone()
    }

    /// Applies `change`, saves `app-settings.json`, and keeps the change only if the save succeeded.
    pub fn update_settings(&self, change: impl FnOnce(&mut AppSettings)) -> anyhow::Result<()> {
        let mut settings = lock(&self.settings);
        let mut next = settings.clone();
        change(&mut next);
        updates::save_settings(&self.data_dir, &next)?;
        *settings = next;
        Ok(())
    }

    pub fn phase(&self) -> Phase {
        match &*lock(&self.slot) {
            Slot::Starting => Phase { phase: "starting", error: None },
            Slot::Running(_) => Phase { phase: "running", error: None },
            Slot::Failed(e) => Phase { phase: "failed", error: Some(e.clone()) },
            Slot::Stopped => Phase { phase: "stopped", error: None },
        }
    }

    /// The running engine, or a user-presentable reason why there is none.
    pub fn engine(&self) -> Result<Arc<Engine>, String> {
        match &*lock(&self.slot) {
            Slot::Running(engine) => Ok(Arc::clone(engine)),
            Slot::Starting => Err("The engine is still starting.".into()),
            Slot::Failed(e) => Err(format!("The engine failed to start: {e}")),
            Slot::Stopped => Err("The engine is stopped.".into()),
        }
    }

    fn running_engine(&self) -> Option<Arc<Engine>> {
        self.engine().ok()
    }

    /// Starts the engine on a background thread. Must not be called while an engine is running.
    pub fn start(self: &Arc<Self>, app: &AppHandle) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *lock(&self.slot) = Slot::Starting;
        let sup = Arc::clone(self);
        let app = app.clone();
        let spawned = thread::Builder::new().name("engine-start".into()).spawn(move || {
            match Engine::start(sup.data_dir.clone()) {
                Ok(engine) => {
                    spawn_forwarder(&app, &sup, &engine, generation);
                    eprintln!("facebridge: engine started (data dir {})", sup.data_dir.display());
                    *lock(&sup.slot) = Slot::Running(engine);
                    sup.wake_update_checker();
                }
                Err(e) => {
                    let message = format!("{e:#}");
                    eprintln!("facebridge: engine failed to start: {message}");
                    *lock(&sup.slot) = Slot::Failed(message);
                }
            }
            let _ = app.emit(EVENT_ENGINE_STATE, sup.phase());
        });
        match spawned {
            Ok(handle) => *lock(&self.starter) = Some(handle),
            Err(e) => *lock(&self.slot) = Slot::Failed(format!("cannot spawn engine thread: {e}")),
        }
    }

    /// Waits for a pending start, then shuts the engine down (modules get their teardown window). Idempotent.
    pub fn shutdown(&self) {
        if let Some(handle) = lock(&self.starter).take() {
            let _ = handle.join();
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        let previous = std::mem::replace(&mut *lock(&self.slot), Slot::Stopped);
        if let Slot::Running(engine) = previous {
            engine.shutdown();
        }
    }

    /// Stops the engine and starts a fresh one (re-reading `config.json`).
    pub fn restart(self: &Arc<Self>, app: &AppHandle) {
        self.shutdown();
        self.start(app);
    }

    /// Terminates background threads (live pusher); call once at process exit.
    pub fn stop_background(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(t) = self.live_thread.get() {
            t.unpark();
        }
        self.wake_update_checker();
    }

    /// Makes the update checker look at the installed modules now (new install, engine start, switched on).
    pub fn wake_update_checker(&self) {
        if let Some(t) = self.update_thread.get() {
            t.unpark();
        }
    }

    /// Spawns the thread that checks module sources for new releases while the setting is on.
    pub fn spawn_update_checker(self: &Arc<Self>, app: &AppHandle) {
        let sup = Arc::clone(self);
        let app = app.clone();
        let handle = thread::Builder::new().name("module-updates".into()).spawn(move || {
            loop {
                if sup.stopping.load(Ordering::SeqCst) {
                    return;
                }
                if sup.updates.enabled()
                    && let Some(engine) = sup.running_engine()
                {
                    let repos = engine.snapshot().modules.into_iter().filter_map(|m| m.source_repo).collect();
                    if sup.updates.check(&repos) {
                        let _ = app.emit(EVENT_MODULE_UPDATES, ());
                    }
                }
                thread::park_timeout(UPDATE_POLL);
            }
        });
        if let Ok(handle) = handle {
            let _ = self.update_thread.set(handle.thread().clone());
        }
    }

    pub fn set_live(&self, live: bool) {
        self.live.store(live, Ordering::SeqCst);
        if live && let Some(t) = self.live_thread.get() {
            t.unpark();
        }
    }

    pub fn set_live_raw(&self, raw: bool) {
        self.live_raw.store(raw, Ordering::SeqCst);
    }

    /// Spawns the thread that pushes `frame` events (only while the Live view is open and the window visible).
    pub fn spawn_live_pusher(self: &Arc<Self>, app: &AppHandle) {
        let sup = Arc::clone(self);
        let app = app.clone();
        let handle = thread::Builder::new().name("live-frames".into()).spawn(move || {
            let mut last: Option<FrameView> = None;
            loop {
                if sup.stopping.load(Ordering::SeqCst) {
                    return;
                }
                let visible = app
                    .get_webview_window("main")
                    .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
                if !sup.live.load(Ordering::SeqCst) || !visible {
                    last = None;
                    thread::park_timeout(Duration::from_millis(500));
                    continue;
                }
                if let Some(engine) = sup.running_engine() {
                    let (frame, claims) =
                        if sup.live_raw.load(Ordering::SeqCst) { engine.latest_raw_frame() } else { engine.latest_frame() };
                    let view = FrameView::new(&frame, claims.eye, claims.expression);
                    if last.as_ref() != Some(&view) {
                        let _ = app.emit(EVENT_FRAME, &view);
                        last = Some(view);
                    }
                }
                thread::sleep(FRAME_INTERVAL);
            }
        });
        if let Ok(handle) = handle {
            let _ = self.live_thread.set(handle.thread().clone());
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum EngineEventView {
    ModulesChanged,
    NotificationsChanged,
    AvatarChanged,
    OscStatusChanged,
    MutationChanged,
    Log { entry: fb_core::LogEntry },
    Notice { notification: fb_core::NotificationView },
}

fn spawn_forwarder(app: &AppHandle, sup: &Arc<Supervisor>, engine: &Arc<Engine>, generation: u64) {
    let rx = engine.subscribe();
    let app = app.clone();
    let sup = Arc::clone(sup);
    let _ = thread::Builder::new().name("engine-events".into()).spawn(move || {
        loop {
            match rx.recv_timeout(FORWARDER_POLL) {
                Ok(event) => {
                    let view = match event {
                        EngineEvent::ModulesChanged => {
                            sup.wake_update_checker();
                            EngineEventView::ModulesChanged
                        }
                        EngineEvent::NotificationsChanged => EngineEventView::NotificationsChanged,
                        EngineEvent::Notice(notification) => EngineEventView::Notice { notification },
                        EngineEvent::AvatarChanged => EngineEventView::AvatarChanged,
                        EngineEvent::OscStatusChanged => EngineEventView::OscStatusChanged,
                        EngineEvent::MutationChanged => EngineEventView::MutationChanged,
                        EngineEvent::Log(entry) => EngineEventView::Log { entry },
                    };
                    let _ = app.emit(EVENT_ENGINE, view);
                }
                Err(e) if e.is_disconnected() => return,
                Err(_) => {
                    if sup.generation.load(Ordering::SeqCst) != generation {
                        return;
                    }
                }
            }
        }
    });
}
