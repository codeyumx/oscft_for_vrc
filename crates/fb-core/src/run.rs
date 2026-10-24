//! One module host process: spawn, handshake, arbitration, frame pump, teardown.

use crate::engine::{Inner, TEARDOWN_TIMEOUT, apply_settings, lock};
use crate::host::{child_command, discover_module_host};
use crate::merge::{LiveCheck, RunKey};
use crate::process::{assign_to_job, pump_lines};
use crate::settings::Schema;
use crate::store;
use crate::types::{EngineEvent, ModuleRunState};
use fb_osc::Claims;
use fb_protocol::{ChildMessage, HostMessage, PROTOCOL_VERSION, ProtocolError, read_child_message, write_host_message};
use serde_json::Value;
use std::io::{self, BufReader};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The host must connect and say `Hello` within this long of being spawned.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Time for the module assembly to load (`Loaded`) and to finish `Initialize` (`InitResult`).
const LOADED_TIMEOUT: Duration = Duration::from_secs(30);
const INIT_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Control block of one running module host; shared between its thread and the engine API.
pub(crate) struct RunCtl {
    pub key: RunKey,
    /// Started by the start sequence (so later modules will still be offered whatever this one releases), not on
    /// its own by an install / enable.
    pub in_sequence: bool,
    /// The engine asked this run to stop (stop, disable, uninstall, restart, shutdown).
    pub stop: AtomicBool,
    /// `Init` is about to be (or has been) sent: settings changes are pushed to the child from now on.
    pub init_sent: AtomicBool,
    writer: Mutex<Option<TcpStream>>,
    child: Mutex<Option<Child>>,
    done: (Mutex<bool>, Condvar),
}

impl RunCtl {
    fn new(key: RunKey, in_sequence: bool) -> Self {
        Self {
            key,
            in_sequence,
            stop: AtomicBool::new(false),
            init_sent: AtomicBool::new(false),
            writer: Mutex::new(None),
            child: Mutex::new(None),
            done: (Mutex::new(false), Condvar::new()),
        }
    }

    pub fn send(&self, m: &HostMessage) -> io::Result<()> {
        match lock(&self.writer).as_mut() {
            Some(w) => write_host_message(w, m),
            None => Err(io::Error::from(io::ErrorKind::NotConnected)),
        }
    }

    /// Asks the run to end: the flag stops a pending spawn, `Teardown` ends a connected child.
    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.send(&HostMessage::Teardown);
    }

    fn kill(&self) {
        if let Some(c) = lock(&self.child).as_mut() {
            let _ = c.kill();
        }
        self.close_connection();
    }

    fn close_connection(&self) {
        if let Some(w) = lock(&self.writer).take() {
            let _ = w.shutdown(Shutdown::Both);
        }
    }

    fn wait_done(&self, timeout: Duration) -> bool {
        let (flag, cv) = &self.done;
        let guard = lock(flag);
        let (guard, _) = cv.wait_timeout_while(guard, timeout, |done| !*done).unwrap_or_else(|e| e.into_inner());
        *guard
    }

    fn mark_done(&self) {
        *lock(&self.done.0) = true;
        self.done.1.notify_all();
    }
}

/// Stops one run: `Teardown`, up to 5 s, then kill.
pub(crate) fn stop_run(ctl: &Arc<RunCtl>) {
    stop_many(std::slice::from_ref(ctl));
}

/// Stops several runs concurrently: all get `Teardown` at once and share one 5 s grace period.
pub(crate) fn stop_many(ctls: &[Arc<RunCtl>]) {
    for c in ctls {
        c.request_stop();
    }
    let deadline = Instant::now() + TEARDOWN_TIMEOUT;
    for c in ctls {
        if !c.wait_done(deadline.saturating_duration_since(Instant::now())) {
            c.kill();
            c.wait_done(Duration::from_secs(3));
        }
    }
}

/// Invalidates in-flight start sequences and returns the runs to stop.
pub(crate) fn bump_generation(inner: &Inner) -> (u64, Vec<Arc<RunCtl>>) {
    let mut st = lock(&inner.state);
    st.generation += 1;
    (st.generation, st.modules.values().filter_map(|m| m.run.clone()).collect())
}

/// Starts all enabled modules, one at a time, in priority order, on a background thread.
pub(crate) fn spawn_start_sequence(inner: &Arc<Inner>, generation: u64) {
    if inner.offline {
        return;
    }
    let inner = Arc::clone(inner);
    let _ = std::thread::Builder::new().name("fb-start-sequence".into()).spawn(move || {
        let ids: Vec<String> = {
            let st = lock(&inner.state);
            st.config.modules.iter().filter(|m| m.enabled).map(|m| m.id.clone()).collect()
        };
        for id in ids {
            if lock(&inner.state).generation != generation {
                return;
            }
            start_module(&inner, &id, generation, true);
        }
    });
}

/// Starts a single module on a background thread (install / enable while the engine is running).
pub(crate) fn spawn_hot_start(inner: &Arc<Inner>, id: &str, generation: u64) {
    if inner.offline {
        return;
    }
    let inner = Arc::clone(inner);
    let id = id.to_owned();
    let _ = std::thread::Builder::new()
        .name("fb-hot-start".into())
        .spawn(move || start_module(&inner, &id, generation, false));
}

/// Signals "startup finished" (running, unclaimed or failed) to whoever is waiting to start the next module.
struct StartupSignal(Option<mpsc::Sender<()>>);

impl StartupSignal {
    fn done(&mut self) {
        self.0.take();
    }
}

/// Spawns the module host for `id` and blocks until it is running, unclaimed or failed.
fn start_module(inner: &Arc<Inner>, id: &str, generation: u64, in_sequence: bool) {
    let _startup = lock(&inner.startup);
    let (ctl, host_cfg, dotnet_root) = {
        let mut st = lock(&inner.state);
        let enabled = st.config.modules.iter().any(|m| m.id == id && m.enabled);
        if st.generation != generation || inner.shutting_down.load(Ordering::SeqCst) || !enabled {
            return;
        }
        let (host_cfg, dotnet_root) = (st.config.module_host.clone(), st.config.dotnet_root.clone());
        let Some(rt) = st.modules.get_mut(id) else { return };
        if rt.run.is_some() {
            return;
        }
        let ctl = Arc::new(RunCtl::new(inner.next_run_key(), in_sequence));
        rt.run = Some(Arc::clone(&ctl));
        rt.state = ModuleRunState::Starting;
        rt.supports = Claims::default();
        rt.claims = Claims::default();
        rt.status = None;
        rt.last_error = None;
        rt.images.clear();
        rt.init_name = None;
        (ctl, host_cfg, dotnet_root)
    };
    inner.emit(EngineEvent::ModulesChanged);

    let (tx, rx) = mpsc::channel();
    let thread = {
        let inner = Arc::clone(inner);
        let ctl = Arc::clone(&ctl);
        let id = id.to_owned();
        std::thread::Builder::new().name(format!("fb-module-{id}")).spawn(move || {
            run_module(&inner, &id, &ctl, host_cfg.as_deref(), dotnet_root.as_deref(), StartupSignal(Some(tx)));
        })
    };
    match thread {
        Ok(_) => {
            // Returns when the module thread signals or ends (the sender is dropped either way).
            let _ = rx.recv();
        }
        Err(e) => {
            inner.update_module(id, |rt| {
                rt.state = ModuleRunState::Failed;
                rt.last_error = Some(format!("could not start module thread: {e}"));
                rt.run = None;
            });
        }
    }
}

enum Incoming {
    Msg(ChildMessage),
    /// The connection ended; `Some` carries a protocol/I/O error, `None` a clean close.
    End(Option<String>),
}

enum End {
    /// `TeardownDone` received.
    TeardownDone,
    /// Connection dropped (the module host died or was killed).
    Closed(Option<String>),
    /// The module claimed nothing and was torn down.
    Unclaimed,
    /// None of its claims showed live data in the startup check; it was torn down (`last_error` says so).
    NoData,
}

fn run_module(
    inner: &Arc<Inner>,
    id: &str,
    ctl: &Arc<RunCtl>,
    host_cfg: Option<&std::path::Path>,
    dotnet_root: Option<&std::path::Path>,
    mut signal: StartupSignal,
) {
    let result = drive(inner, id, ctl, host_cfg, dotnet_root, &mut signal);
    signal.done();

    // The host is on its way out (or must be made to be): wait a moment, then kill.
    let grace = match &result {
        Ok(End::TeardownDone | End::Unclaimed | End::NoData) => TEARDOWN_TIMEOUT,
        _ => Duration::from_secs(2),
    };
    let exit = reap(ctl, grace);
    ctl.close_connection();

    let requested = ctl.stop.load(Ordering::SeqCst);
    let (state, error) = match result {
        Ok(End::Unclaimed) => (ModuleRunState::Unclaimed, None),
        Ok(End::NoData) => (ModuleRunState::NoData, None),
        _ if requested => (ModuleRunState::Stopped, None),
        Ok(End::TeardownDone) => (ModuleRunState::Exited, None),
        Ok(End::Closed(err)) => match (exit, err) {
            (Some(0), _) => (ModuleRunState::Exited, None),
            (Some(code), err) => (ModuleRunState::Failed, Some(with_detail(describe_exit(code), err))),
            (None, err) => (ModuleRunState::Failed, Some(with_detail("lost connection to the module host".into(), err))),
        },
        Err(e) => (ModuleRunState::Failed, Some(match exit {
            Some(code) if code != 0 => format!("{e} ({})", describe_exit(code)),
            _ => e,
        })),
    };

    {
        let mut merger = lock(&inner.merger);
        if let Some((frame, claims)) = merger.release(ctl.key) {
            inner.submit(&frame, claims);
        }
    }
    let cleared = lock(&inner.state).notifications.clear_module(id);
    let name = inner.update_module(id, |rt| {
        if rt.run.as_ref().is_some_and(|r| r.key == ctl.key) {
            rt.run = None;
            rt.state = state;
            rt.claims = Claims::default();
            rt.status = None;
            if error.is_some() {
                rt.last_error = error.clone();
            }
        }
        rt.display_name(id)
    });
    if cleared {
        inner.emit(EngineEvent::NotificationsChanged);
    }
    if let (Some(name), Some(error)) = (name, &error) {
        inner.log(Some(id), "error", "engine", format!("Module {name}: {error}"));
    }
    ctl.mark_done();
}

fn with_detail(base: String, detail: Option<String>) -> String {
    match detail {
        Some(d) => format!("{base}: {d}"),
        None => base,
    }
}

fn describe_exit(code: i32) -> String {
    match code {
        2 => "module host rejected its arguments (exit code 2)".into(),
        3 => "module host could not connect or complete the handshake (exit code 3)".into(),
        4 => "module failed to load (exit code 4)".into(),
        5 => "module crashed with an unhandled exception (exit code 5)".into(),
        6 => "module host lost its parent process (exit code 6)".into(),
        n => format!("module host exited with code {n}"),
    }
}

/// Waits up to `grace` for the child to exit, then kills it. Returns its exit code if known.
fn reap(ctl: &RunCtl, grace: Duration) -> Option<i32> {
    let deadline = Instant::now() + grace;
    loop {
        {
            let mut guard = lock(&ctl.child);
            let child = guard.as_mut()?;
            match child.try_wait() {
                Ok(Some(status)) => return status.code(),
                Ok(None) => {}
                Err(_) => return None,
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                return child.wait().ok().and_then(|s| s.code());
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn drive(
    inner: &Arc<Inner>,
    id: &str,
    ctl: &Arc<RunCtl>,
    host_cfg: Option<&std::path::Path>,
    dotnet_root: Option<&std::path::Path>,
    signal: &mut StartupSignal,
) -> Result<End, String> {
    // --- spawn -----------------------------------------------------------------------------------------------
    let (executable, dll_name) = lock(&inner.state)
        .modules
        .get(id)
        .map(|m| (m.meta.executable.clone(), m.meta.dll_file_name.clone()))
        .unwrap_or_default();
    let module_dir = inner.paths.module_dir(id);
    // A native module is its own program; only .NET modules need (and start) the module host.
    let (program, mut cmd) = match (executable, dll_name) {
        (Some(exe), _) => {
            let path = store::native_executable(&module_dir, &exe);
            if !path.is_file() {
                return Err(format!("module executable {exe} is missing"));
            }
            let mut cmd = child_command(&path, None);
            cmd.current_dir(&module_dir);
            (path, cmd)
        }
        (None, Some(dll_name)) => {
            let dll = module_dir.join(&dll_name);
            if !dll.is_file() {
                return Err(format!("module DLL {dll_name} is missing"));
            }
            let host = discover_module_host(host_cfg, &inner.paths.data_dir).map_err(|e| format!("{e:#}"))?;
            let mut cmd = child_command(&host, dotnet_root);
            cmd.arg("--module").arg(&dll);
            (host, cmd)
        }
        (None, None) => return Err("module.json has neither Executable nor DllFileName".into()),
    };
    let data_dir = inner.paths.module_data_dir(id);
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("cannot open a local port: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = Uuid::new_v4().simple().to_string();

    let mut child = cmd
        .arg("--connect")
        .arg(format!("127.0.0.1:{port}"))
        .arg("--token")
        .arg(&token)
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .env("FACEBRIDGE_USER_DATA_DIR", &inner.paths.data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", program.display()))?;
    if let Err(e) = assign_to_job(&child) {
        inner.log(Some(id), "warning", "engine", format!("Could not add the module host to the kill-on-close job: {e}"));
    }
    spawn_pump(child.stdout.take(), inner, id, "information", "stdout");
    spawn_pump(child.stderr.take(), inner, id, "warning", "stderr");
    *lock(&ctl.child) = Some(child);

    // --- accept + Hello --------------------------------------------------------------------------------------
    let stream = accept_hello(ctl, &listener, &token)?;
    drop(listener);
    let _ = stream.set_nodelay(true);
    let writer = stream.try_clone().map_err(|e| e.to_string())?;
    let _ = writer.set_write_timeout(Some(WRITE_TIMEOUT));
    *lock(&ctl.writer) = Some(writer);
    if ctl.stop.load(Ordering::SeqCst) {
        let _ = ctl.send(&HostMessage::Teardown);
    }
    let rx = spawn_reader(stream);

    // --- Loaded ----------------------------------------------------------------------------------------------
    let (module_type, supports) = match wait_message(inner, id, ctl, &rx, LOADED_TIMEOUT, "load")? {
        ChildMessage::Loaded { module_type, supports_eye, supports_expression } => {
            (module_type, Claims { eye: supports_eye, expression: supports_expression })
        }
        ChildMessage::LoadFailed { reason } => return Err(format!("module failed to load: {reason}")),
        other => return Err(format!("protocol error: expected Loaded, got {}", message_name(&other))),
    };
    inner.update_module(id, |rt| {
        rt.supports = supports;
        rt.supports_known = true;
        rt.state = ModuleRunState::Starting;
    });
    inner.log(
        Some(id),
        "information",
        "engine",
        format!("Loaded {module_type} (eye: {}, expression: {})", supports.eye, supports.expression),
    );

    // --- arbitration -----------------------------------------------------------------------------------------
    let Some(offer) = lock(&inner.merger).offer(supports) else {
        inner.update_module(id, |rt| rt.state = ModuleRunState::Unclaimed);
        inner.log(Some(id), "information", "engine", "Nothing left to claim for this module; it was not started");
        return finish_unclaimed(inner, id, ctl, &rx, signal, End::Unclaimed);
    };

    // Stored settings go first so the module sees them during `Initialize`.
    ctl.init_sent.store(true, Ordering::SeqCst);
    let stored = {
        let st = lock(&inner.state);
        st.modules.get(id).filter(|m| m.stored).map(|m| Value::Object(m.persisted_values()).to_string())
    };
    if let Some(json) = stored {
        ctl.send(&HostMessage::Settings { json }).map_err(|e| format!("lost the module host while sending settings: {e}"))?;
    }
    ctl.send(&HostMessage::Init { eye_available: offer.eye, expression_available: offer.expression })
        .map_err(|e| format!("lost the module host while sending Init: {e}"))?;
    inner.update_module(id, |rt| rt.state = ModuleRunState::Initializing);

    let (claimed, name, images) = match wait_message(inner, id, ctl, &rx, INIT_TIMEOUT, "initialize")? {
        ChildMessage::InitResult { eye, expression, name, images } => (Claims { eye, expression }, name, images),
        other => return Err(format!("protocol error: expected InitResult, got {}", message_name(&other))),
    };
    let granted = lock(&inner.merger).claim(ctl.key, offer, claimed);
    inner.update_module(id, |rt| {
        rt.init_name = Some(name).filter(|n| !n.is_empty());
        rt.images = images;
        rt.claims = granted;
    });
    if !granted.eye && !granted.expression {
        inner.update_module(id, |rt| rt.state = ModuleRunState::Unclaimed);
        inner.log(Some(id), "information", "engine", "The module claimed nothing; it was torn down");
        return finish_unclaimed(inner, id, ctl, &rx, signal, End::Unclaimed);
    }
    let kept = match check_live_data(inner, id, ctl, &rx, granted) {
        Probe::Kept(kept) => kept,
        Probe::Ended(end) => return Ok(end),
    };
    if !kept.eye && !kept.expression {
        return finish_unclaimed(inner, id, ctl, &rx, signal, End::NoData);
    }
    inner.update_module(id, |rt| {
        rt.claims = kept;
        rt.state = ModuleRunState::Running;
    });
    inner.log(
        Some(id),
        "information",
        "engine",
        format!("Running (claims: eye {}, expression {})", kept.eye, kept.expression),
    );
    signal.done();

    // --- frames ----------------------------------------------------------------------------------------------
    loop {
        match rx.recv() {
            Ok(Incoming::Msg(ChildMessage::Frame(f))) => {
                let mut merger = lock(&inner.merger);
                if let Some((frame, claims)) = merger.update(ctl.key, f.flags, &f.frame) {
                    inner.submit(&frame, claims);
                }
            }
            Ok(Incoming::Msg(ChildMessage::TeardownDone)) => return Ok(End::TeardownDone),
            Ok(Incoming::Msg(msg)) => {
                if let Some(other) = handle_common(inner, id, ctl, msg) {
                    inner.log(Some(id), "warning", "engine", format!("Ignoring unexpected {}", message_name(&other)));
                }
            }
            Ok(Incoming::End(err)) => return Ok(End::Closed(err)),
            Err(_) => return Ok(End::Closed(None)),
        }
    }
}

/// The module has nothing to do (`end` says why): tell it to tear down and wait (briefly) for it to finish.
fn finish_unclaimed(
    inner: &Arc<Inner>,
    id: &str,
    ctl: &Arc<RunCtl>,
    rx: &crossbeam_channel::Receiver<Incoming>,
    signal: &mut StartupSignal,
    end: End,
) -> Result<End, String> {
    let _ = ctl.send(&HostMessage::Teardown);
    signal.done();
    let deadline = Instant::now() + TEARDOWN_TIMEOUT;
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Incoming::Msg(ChildMessage::TeardownDone)) | Ok(Incoming::End(_)) | Err(_) => return Ok(end),
            Ok(Incoming::Msg(msg)) => {
                handle_common(inner, id, ctl, msg);
            }
        }
    }
}

fn describe_parts(parts: Claims) -> &'static str {
    match (parts.eye, parts.expression) {
        (true, true) => "eye and expression tracking",
        (true, false) => "eye tracking",
        (false, true) => "expression tracking",
        (false, false) => "nothing",
    }
}

/// Parts a module that is later in the start sequence could still take over: what its `supports` says, or
/// everything while that is not known yet. Only a start sequence offers parts on to later modules; a module that
/// is started alone (install / enable) has nobody to hand a part to.
fn later_takers(inner: &Inner, ctl: &RunCtl, id: &str) -> Claims {
    let mut takers = Claims::default();
    if !ctl.in_sequence {
        return takers;
    }
    let st = lock(&inner.state);
    let Some(pos) = st.config.modules.iter().position(|m| m.id == id) else { return takers };
    for entry in st.config.modules[pos + 1..].iter().filter(|m| m.enabled) {
        let Some(rt) = st.modules.get(&entry.id).filter(|rt| rt.run.is_none()) else { continue };
        takers.eye |= !rt.supports_known || rt.supports.eye;
        takers.expression |= !rt.supports_known || rt.supports.expression;
    }
    takers
}

enum Probe {
    /// The check is over; the module keeps these claims (none: it was marked `NoData` and must be torn down).
    Kept(Claims),
    /// The run ended while waiting.
    Ended(End),
}

/// The startup-only live-data check (see `ModuleOptions::require_data`). Runs after `Init` granted `granted`, while
/// the startup lock is still held. Waits until every part that a later module could take over has shown live data
/// or the module's timeout passes; parts still without data then go back to the unclaimed pool. Claims are never
/// revisited after this returns.
fn check_live_data(
    inner: &Arc<Inner>,
    id: &str,
    ctl: &Arc<RunCtl>,
    rx: &crossbeam_channel::Receiver<Incoming>,
    granted: Claims,
) -> Probe {
    let options = lock(&inner.state).config.modules.iter().find(|m| m.id == id).map(|m| m.options).unwrap_or_default();
    if !options.require_data {
        inner.log(Some(id), "information", "engine", "Live-data check is off for this module; keeping its claims");
        return Probe::Kept(granted);
    }
    let takers = later_takers(inner, ctl, id);
    let watch = Claims { eye: granted.eye && takers.eye, expression: granted.expression && takers.expression };
    let unwatched = Claims { eye: granted.eye && !watch.eye, expression: granted.expression && !watch.expression };
    if unwatched.eye || unwatched.expression {
        inner.log(
            Some(id),
            "information",
            "engine",
            format!("No later module could take over {}; keeping that claim without a data check", describe_parts(unwatched)),
        );
    }
    let mut live = LiveCheck::new(watch);
    let timeout = options.data_timeout();
    if watch.eye || watch.expression {
        inner.log(
            Some(id),
            "information",
            "engine",
            format!("Waiting up to {} s for {} data", timeout.as_secs(), describe_parts(watch)),
        );
    }
    let deadline = Instant::now() + timeout;
    loop {
        let pending = live.pending();
        if !pending.eye && !pending.expression {
            break;
        }
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Incoming::Msg(ChildMessage::Frame(f))) => {
                lock(&inner.merger).update(ctl.key, f.flags, &f.frame);
                live.feed(&f.frame);
            }
            Ok(Incoming::Msg(ChildMessage::TeardownDone)) => return Probe::Ended(End::TeardownDone),
            Ok(Incoming::Msg(msg)) => {
                if let Some(other) = handle_common(inner, id, ctl, msg) {
                    inner.log(Some(id), "warning", "engine", format!("Ignoring unexpected {}", message_name(&other)));
                }
            }
            Ok(Incoming::End(err)) => return Probe::Ended(End::Closed(err)),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return Probe::Ended(End::Closed(None)),
        }
    }

    // Everything still pending after the loop is a part a later module could take (only those are watched).
    let released = live.pending();
    if released.eye || released.expression {
        let kept = Claims { eye: granted.eye && !released.eye, expression: granted.expression && !released.expression };
        let verb = if released.eye && released.expression { "were" } else { "was" };
        let message = format!(
            "No tracking data within {} s; {} {verb} offered to the next module",
            timeout.as_secs(),
            describe_parts(released)
        );
        let none_left = !kept.eye && !kept.expression;
        inner.update_module(id, |rt| {
            rt.claims = kept;
            if none_left {
                rt.state = ModuleRunState::NoData;
                rt.last_error = Some(message.clone());
            }
        });
        inner.log(Some(id), "warning", "engine", message);
        let mut merger = lock(&inner.merger);
        merger.release_parts(ctl.key, released);
        let (frame, claims) = merger.merged();
        inner.submit(&frame, claims);
        return Probe::Kept(kept);
    }
    if watch.eye || watch.expression {
        inner.log(Some(id), "information", "engine", format!("Live {} data received", describe_parts(watch)));
    }
    // Frames that arrived during the check were held back; publish the current state now.
    let merger = lock(&inner.merger);
    let (frame, claims) = merger.merged();
    inner.submit(&frame, claims);
    Probe::Kept(granted)
}

/// Accepts connections until one starts with a valid `Hello`; other connections are dropped.
fn accept_hello(ctl: &RunCtl, listener: &TcpListener, token: &str) -> Result<TcpStream, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + HELLO_TIMEOUT;
    let mut rejected: Option<String> = None;
    let mut buf = Vec::new();
    loop {
        if ctl.stop.load(Ordering::SeqCst) {
            ctl.kill();
            return Err("stopped before the module host connected".into());
        }
        // Sampled before accepting: a connection made before the child exited is already queued, so a child that
        // connects, fails fast and exits is still seen.
        let exited = lock(&ctl.child).as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(Some(_))));
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let left = deadline.saturating_duration_since(Instant::now()).clamp(Duration::from_millis(50), Duration::from_secs(2));
                let _ = stream.set_read_timeout(Some(left));
                match read_child_message(&mut stream, &mut buf) {
                    Ok(ChildMessage::Hello { version, token: t, .. }) if t == token && version == PROTOCOL_VERSION => {
                        let _ = stream.set_read_timeout(None);
                        return Ok(stream);
                    }
                    Ok(ChildMessage::Hello { version, token: t, .. }) if t == token => {
                        rejected = Some(format!("protocol version {version} (expected {PROTOCOL_VERSION})"));
                    }
                    _ => {}
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if exited {
                    return Err(with_detail("module host exited before connecting".into(), rejected));
                }
                if Instant::now() >= deadline {
                    ctl.kill();
                    return Err(with_detail("module host did not connect within 10 s".into(), rejected));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(format!("accept failed: {e}")),
        }
    }
}

fn spawn_reader(stream: TcpStream) -> crossbeam_channel::Receiver<Incoming> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let _ = std::thread::Builder::new().name("fb-module-reader".into()).spawn(move || {
        let mut reader = BufReader::with_capacity(16 * 1024, stream);
        let mut buf = Vec::new();
        loop {
            let item = match read_child_message(&mut reader, &mut buf) {
                Ok(m) => Incoming::Msg(m),
                Err(ProtocolError::Closed) => Incoming::End(None),
                Err(e) => Incoming::End(Some(e.to_string())),
            };
            let last = matches!(item, Incoming::End(_));
            if tx.send(item).is_err() || last {
                return;
            }
        }
    });
    rx
}

/// Waits for the next handshake message (anything else, like logs and notifications, is handled on the way).
fn wait_message(
    inner: &Arc<Inner>,
    id: &str,
    ctl: &Arc<RunCtl>,
    rx: &crossbeam_channel::Receiver<Incoming>,
    timeout: Duration,
    phase: &str,
) -> Result<ChildMessage, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Incoming::Msg(msg)) => {
                if let Some(msg) = handle_common(inner, id, ctl, msg) {
                    return Ok(msg);
                }
            }
            Ok(Incoming::End(err)) => {
                return Err(with_detail(format!("module host closed the connection during {phase}"), err));
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                ctl.kill();
                return Err(format!("module did not {phase} within {} s", timeout.as_secs()));
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                return Err(format!("module host closed the connection during {phase}"));
            }
        }
    }
}

fn message_name(m: &ChildMessage) -> &'static str {
    match m {
        ChildMessage::Hello { .. } => "Hello",
        ChildMessage::Loaded { .. } => "Loaded",
        ChildMessage::LoadFailed { .. } => "LoadFailed",
        ChildMessage::InitResult { .. } => "InitResult",
        ChildMessage::Frame(_) => "Frame",
        ChildMessage::Log { .. } => "Log",
        ChildMessage::Notify { .. } => "Notify",
        ChildMessage::NotifyMessage { .. } => "NotifyMessage",
        ChildMessage::NotifyClear { .. } => "NotifyClear",
        ChildMessage::Status { .. } => "Status",
        ChildMessage::SettingsSchema { .. } => "SettingsSchema",
        ChildMessage::SettingsWrite { .. } => "SettingsWrite",
        ChildMessage::TeardownDone => "TeardownDone",
    }
}

/// Handles messages that are valid in any phase; returns the message back if it is not one of them.
fn handle_common(inner: &Arc<Inner>, id: &str, ctl: &Arc<RunCtl>, msg: ChildMessage) -> Option<ChildMessage> {
    match msg {
        ChildMessage::Log { level, category, message, exception, .. } => {
            let text = if exception.is_empty() { message } else { format!("{message}\n{exception}") };
            inner.log(Some(id), level.as_str(), &category, text);
        }
        ChildMessage::Notify { level, key, message } => inner.notify(id, level.as_str(), &key, &message, false),
        ChildMessage::NotifyMessage { level, key, message } => inner.notify(id, level.as_str(), &key, &message, true),
        ChildMessage::NotifyClear { key } => inner.clear_notifications(id, &key),
        ChildMessage::Status { state } => {
            inner.update_module(id, |rt| rt.status = Some(state));
        }
        ChildMessage::SettingsSchema { json } => on_schema(inner, id, ctl, &json),
        ChildMessage::SettingsWrite { json } => match serde_json::from_str::<Value>(&json) {
            Ok(Value::Object(patch)) => {
                if let Err(e) = apply_settings(inner, id, &patch, false) {
                    inner.log(Some(id), "warning", "engine", format!("Could not store the module's settings: {e:#}"));
                }
            }
            _ => inner.log(Some(id), "warning", "engine", "Ignoring SettingsWrite that is not a JSON object"),
        },
        ChildMessage::Hello { .. } => {}
        other => return Some(other),
    }
    None
}

fn on_schema(inner: &Arc<Inner>, id: &str, ctl: &Arc<RunCtl>, json: &str) {
    let schema = match Schema::parse(json) {
        Ok(s) => s,
        Err(e) => {
            inner.log(Some(id), "warning", "engine", format!("Ignoring invalid settings schema: {e}"));
            return;
        }
    };
    if let Err(e) = crate::config::write_json_atomic(&inner.paths.schema_cache_path(id), &schema.raw) {
        inner.log(Some(id), "warning", "engine", format!("Could not cache the settings schema: {e:#}"));
    }
    let push = inner.update_module(id, |rt| {
        rt.values = schema.declare(&rt.values);
        rt.schema = Some(schema);
        rt.settings_rev += 1;
        rt.stored.then(|| Value::Object(rt.persisted_values()).to_string())
    });
    if let Some(Some(json)) = push {
        if ctl.init_sent.load(Ordering::SeqCst) {
            let _ = ctl.send(&HostMessage::Settings { json });
        }
    }
}

/// Logs each line the child writes to a pipe.
fn spawn_pump<R: io::Read + Send + 'static>(
    stream: Option<R>,
    inner: &Arc<Inner>,
    id: &str,
    level: &'static str,
    category: &'static str,
) {
    if let Some(stream) = stream {
        let inner = Arc::clone(inner);
        let id = id.to_owned();
        std::thread::spawn(move || pump_lines(stream, |line| inner.log(Some(&id), level, category, line)));
    }
}
