//! SDK for native OSCFT modules (`spec/native-modules.md`).
//!
//! A native module is its own program, listed as `"Executable"` in its `module.json`. OSCFT starts it with the
//! launch arguments of `spec/ipc-protocol.md`; `main` calls [`run`], which connects back to OSCFT and drives a
//! [`Module`]. No .NET runtime is involved. Native modules only run in OSCFT, not in VRCFaceTracking.
//!
//! ```no_run
//! use fb_module_sdk::{Claims, Context, Expression, Init, Module, TrackingFrame};
//! use std::time::Duration;
//!
//! struct Jaw;
//!
//! impl Module for Jaw {
//!     fn supported(&self) -> Claims {
//!         Claims { eye: false, expression: true }
//!     }
//!     fn initialize(&mut self, _ctx: &Context, available: Claims) -> Init {
//!         Init::new(available)
//!     }
//!     fn run(&mut self, ctx: &Context) {
//!         let mut frame = TrackingFrame::default();
//!         while ctx.sleep(Duration::from_millis(10)) {
//!             frame.set_shape(Expression::JawOpen, 0.5);
//!             ctx.send_frame(&frame);
//!         }
//!     }
//! }
//!
//! fn main() {
//!     fb_module_sdk::run(|_data_dir| Ok(Jaw));
//! }
//! ```

pub use fb_protocol::LogLevel;
pub use fb_tracking::{Expression, Eyes, Head, SHAPE_COUNT, SingleEye, TrackingFrame};
pub use serde_json::{Map, Value};

use fb_protocol::{
    ChildMessage, FrameFlags, FrameMessage, HostMessage, PROTOCOL_VERSION, ProtocolError, encode_frame,
    read_host_message, write_child_message,
};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

/// Process exit codes (`spec/ipc-protocol.md`).
mod exit {
    pub const OK: i32 = 0;
    pub const BAD_ARGUMENTS: i32 = 2;
    pub const CONNECT_FAILED: i32 = 3;
    pub const LOAD_FAILED: i32 = 4;
    pub const CRASHED: i32 = 5;
}

/// How long a stop request waits for [`Module::run`] to return (OSCFT kills the process after 5 s).
const RUN_STOP_TIMEOUT: Duration = Duration::from_secs(4);
/// How long the SDK waits for OSCFT to close its side after the module's last message.
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
/// Frames are at least this far apart (`spec/ipc-protocol.md`).
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(1);

/// Which tracking parts (eye, expression) a module provides or takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Claims {
    pub eye: bool,
    pub expression: bool,
}

impl Claims {
    fn intersect(self, other: Self) -> Self {
        Self { eye: self.eye && other.eye, expression: self.expression && other.expression }
    }

    fn flags(self) -> FrameFlags {
        let mut flags = FrameFlags::default();
        if self.eye {
            flags = flags | FrameFlags::EYE;
        }
        if self.expression {
            flags = flags | FrameFlags::EXPRESSION;
        }
        flags
    }
}

/// What [`Module::initialize`] returns.
#[derive(Clone, Debug, Default)]
pub struct Init {
    /// Parts the module takes. Parts that were not offered are dropped.
    pub claims: Claims,
    /// Name shown in OSCFT; empty keeps the `module.json` `ModuleName`.
    pub name: String,
    /// Images (usually PNG) shown with the module.
    pub images: Vec<Vec<u8>>,
}

impl Init {
    pub fn new(claims: Claims) -> Self {
        Self { claims, ..Self::default() }
    }
}

/// Module status shown in OSCFT (VRCFaceTracking `ModuleState` numbering).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i8)]
pub enum Status {
    Uninitialized = -1,
    Idle = 0,
    Active = 1,
}

/// Something OSCFT sent while the module runs; see [`Context::try_event`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The user changed setting values; read them with [`Context::settings`].
    SettingsChanged,
    /// The user pressed the settings `button` field with this key.
    Action(String),
}

/// A native module. [`run`] calls `supported`, then (once OSCFT decided what to offer) `initialize` and `run` on
/// a dedicated thread, then `teardown` when OSCFT stops the module.
pub trait Module: Send + 'static {
    /// Parts this module can provide. OSCFT only offers parts no other module has claimed.
    fn supported(&self) -> Claims;

    /// Connects to the device. `available` is what OSCFT offers; claim a subset in the returned [`Init`]. Stored
    /// setting values are already in [`Context::settings`].
    fn initialize(&mut self, ctx: &Context, available: Claims) -> Init;

    /// Sends frames with [`Context::send_frame`] until [`Context::is_stopping`]. Only called when something was
    /// claimed. Returning early stops tracking; the process stays up until OSCFT stops the module.
    fn run(&mut self, ctx: &Context);

    /// Releases the device. Called once after `run` returned, when OSCFT stops an initialized module.
    fn teardown(&mut self, _ctx: &Context) {}
}

/// Entry point of a native module's `main`: parses the launch arguments, connects to OSCFT, builds the module with
/// `load` (given the module's writable data directory) and drives it until OSCFT stops it. A `load` error is
/// reported to OSCFT as a load failure. Never returns: the process exits with the protocol's exit code.
pub fn run<M: Module>(load: impl FnOnce(&Path) -> Result<M, String>) -> ! {
    let code = serve(std::env::args().skip(1).collect(), load);
    std::process::exit(code)
}

/// The module's handle to OSCFT, passed to every [`Module`] call. All methods are cheap and never block for long;
/// once the connection is gone they do nothing and [`Context::is_stopping`] is true.
pub struct Context {
    shared: Arc<Shared>,
    events: mpsc::Receiver<Event>,
    data_dir: PathBuf,
    flags: Cell<FrameFlags>,
    seq: Cell<u64>,
    last_frame: Cell<Option<Instant>>,
    buf: RefCell<Vec<u8>>,
    status: Cell<Option<Status>>,
}

impl Context {
    /// The module's own writable directory (kept across updates, removed on uninstall).
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// OSCFT asked the module to stop (or the connection is gone): return from [`Module::run`].
    pub fn is_stopping(&self) -> bool {
        self.shared.is_stopping()
    }

    /// Sleeps up to `duration`, waking early on a stop request. Returns `true` while the module should keep running.
    pub fn sleep(&self, duration: Duration) -> bool {
        !self.shared.wait_stopping(Some(duration))
    }

    /// Sends one tracking frame. Eye/expression data counts only for the parts the module claimed; head data is
    /// sent when any head field is non-zero. Frames closer than 1 ms are delayed to keep that spacing. Returns
    /// `false` when the frame could not be sent (stopping or connection lost).
    pub fn send_frame(&self, frame: &TrackingFrame) -> bool {
        let mut flags = self.flags.get();
        if flags.bits() == 0 || self.is_stopping() {
            return false;
        }
        let h = &frame.head;
        if [h.yaw, h.pitch, h.roll, h.pos_x, h.pos_y, h.pos_z].iter().any(|v| *v != 0.0) {
            flags = flags | FrameFlags::HEAD;
        }
        if let Some(wait) = self.last_frame.get().and_then(|t| MIN_FRAME_INTERVAL.checked_sub(t.elapsed())) {
            std::thread::sleep(wait);
        }
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        let mut buf = self.buf.borrow_mut();
        buf.clear();
        encode_frame(&FrameMessage { seq, flags, frame: *frame }, SHAPE_COUNT, &mut buf)
            .expect("a SHAPE_COUNT frame always encodes");
        let sent = self.shared.send_bytes(&buf);
        self.last_frame.set(Some(Instant::now()));
        sent
    }

    /// Writes a line to the OSCFT log for this module.
    pub fn log(&self, level: LogLevel, message: impl Into<String>) {
        self.shared.log(level, message.into());
    }

    /// Shows an informational pop-up that disappears by itself; nothing is kept on the Home page.
    pub fn notify(&self, level: LogLevel, key: &str, message: impl Into<String>) {
        self.shared.send(&ChildMessage::Notify { level, key: key.to_owned(), message: message.into() });
    }

    /// Shows a message: a pop-up plus an entry on the Home page that stays until the user dismisses it or the
    /// module clears it. Use it for things the user must act on. The same non-empty `key` replaces this module's
    /// previous message; an empty `key` deduplicates by message text.
    pub fn message(&self, level: LogLevel, key: &str, message: impl Into<String>) {
        self.shared.send(&ChildMessage::NotifyMessage { level, key: key.to_owned(), message: message.into() });
    }

    /// Removes this module's message with `key`; an empty `key` removes all of them.
    pub fn clear_message(&self, key: &str) {
        self.shared.send(&ChildMessage::NotifyClear { key: key.to_owned() });
    }

    /// Sets the status shown in OSCFT. Repeating the current status sends nothing.
    pub fn set_status(&self, status: Status) {
        if self.status.replace(Some(status)) != Some(status) {
            self.shared.send(&ChildMessage::Status { state: status as i8 });
        }
    }

    /// Declares the settings form (schema JSON of `spec/ipc-protocol.md`); may be called again to change it.
    pub fn declare_settings(&self, schema: &Value) {
        self.shared.send(&ChildMessage::SettingsSchema { json: schema.to_string() });
    }

    /// Changes setting values from the module side (e.g. `text` status fields); OSCFT stores and shows them.
    pub fn write_settings(&self, values: Map<String, Value>) {
        let json = Value::Object(values.clone()).to_string();
        self.shared.settings().extend(values);
        self.shared.send(&ChildMessage::SettingsWrite { json });
    }

    /// Current setting values (key → value), as last sent by OSCFT or written by the module.
    pub fn settings(&self) -> Map<String, Value> {
        self.shared.settings().clone()
    }

    /// The next pending [`Event`], if any.
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Waits up to `timeout` for the next [`Event`].
    pub fn wait_event(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

/// State shared by the connection reader and the module thread.
struct Shared {
    writer: Mutex<TcpStream>,
    settings: Mutex<Map<String, Value>>,
    stopping: Mutex<bool>,
    stop_changed: Condvar,
    crashed: AtomicBool,
    category: String,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn send(&self, m: &ChildMessage) -> bool {
        let sent = write_child_message(&mut *lock(&self.writer), m).is_ok();
        if !sent {
            self.stop();
        }
        sent
    }

    fn send_bytes(&self, bytes: &[u8]) -> bool {
        let sent = lock(&self.writer).write_all(bytes).is_ok();
        if !sent {
            self.stop();
        }
        sent
    }

    fn log(&self, level: LogLevel, message: String) {
        self.send(&ChildMessage::Log {
            level,
            event_id: 0,
            category: self.category.clone(),
            message,
            exception: String::new(),
        });
    }

    fn settings(&self) -> MutexGuard<'_, Map<String, Value>> {
        lock(&self.settings)
    }

    fn stop(&self) {
        *lock(&self.stopping) = true;
        self.stop_changed.notify_all();
    }

    fn is_stopping(&self) -> bool {
        *lock(&self.stopping)
    }

    /// Blocks until a stop request or `timeout` (`None`: forever); returns whether stopping.
    fn wait_stopping(&self, timeout: Option<Duration>) -> bool {
        let guard = lock(&self.stopping);
        match timeout {
            Some(t) => *self.stop_changed.wait_timeout_while(guard, t, |s| !*s).unwrap_or_else(PoisonError::into_inner).0,
            None => *self.stop_changed.wait_while(guard, |s| !*s).unwrap_or_else(PoisonError::into_inner),
        }
    }

    /// The module panicked: stop sending and let the reader finish with exit code 5.
    fn crash(&self) {
        self.crashed.store(true, Ordering::SeqCst);
        self.stop();
        let writer = lock(&self.writer);
        let _ = writer.set_read_timeout(Some(CLOSE_TIMEOUT));
        let _ = writer.shutdown(Shutdown::Write);
    }
}

struct Options {
    connect: SocketAddr,
    token: String,
    data_dir: PathBuf,
}

impl Options {
    /// `--connect <ip:port> --token <hex> --data-dir <dir> --parent-pid <pid>`. The parent pid is validated but not
    /// watched: when OSCFT exits, the connection closes and the module stops.
    fn parse(args: &[String]) -> Result<Self, String> {
        let (mut connect, mut token, mut data_dir, mut parent) = (None, None, None, None);
        for pair in args.chunks(2) {
            let [name, value] = pair else { return Err(format!("missing value for {}", pair[0])) };
            let slot = match name.as_str() {
                "--connect" => &mut connect,
                "--token" => &mut token,
                "--data-dir" => &mut data_dir,
                "--parent-pid" => &mut parent,
                other => return Err(format!("unknown argument {other}")),
            };
            *slot = Some(value.clone());
        }
        let (Some(connect), Some(token), Some(data_dir), Some(parent)) = (connect, token, data_dir, parent) else {
            return Err("required: --connect <ip:port> --token <token> --data-dir <dir> --parent-pid <pid>".into());
        };
        let connect = connect.parse().map_err(|_| format!("invalid --connect value '{connect}'"))?;
        if token.is_empty() {
            return Err("empty --token".into());
        }
        let data_dir = PathBuf::from(data_dir);
        if !data_dir.is_absolute() {
            return Err("--data-dir must be an absolute path".into());
        }
        if parent.parse::<u32>().ok().filter(|p| *p > 0).is_none() {
            return Err(format!("invalid --parent-pid value '{parent}'"));
        }
        Ok(Self { connect, token, data_dir })
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

/// Runs one module session with `args` (without the program name); returns the process exit code.
fn serve<M: Module>(args: Vec<String>, load: impl FnOnce(&Path) -> Result<M, String>) -> i32 {
    let options = match Options::parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return exit::BAD_ARGUMENTS;
        }
    };
    let connected = TcpStream::connect(options.connect).and_then(|s| {
        s.set_nodelay(true)?;
        let reader = s.try_clone()?;
        Ok((s, reader))
    });
    let (writer, stream) = match connected {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("cannot connect to OSCFT at {}: {e}", options.connect);
            return exit::CONNECT_FAILED;
        }
    };
    let shared = Arc::new(Shared {
        writer: Mutex::new(writer),
        settings: Mutex::new(Map::new()),
        stopping: Mutex::new(false),
        stop_changed: Condvar::new(),
        crashed: AtomicBool::new(false),
        category: std::any::type_name::<M>().to_owned(),
    });
    let hello = ChildMessage::Hello { version: PROTOCOL_VERSION, token: options.token, pid: std::process::id() };
    if !shared.send(&hello) {
        eprintln!("cannot send Hello to OSCFT");
        return exit::CONNECT_FAILED;
    }

    let loaded = panic::catch_unwind(AssertUnwindSafe(|| {
        let module = load(&options.data_dir)?;
        let supported = module.supported();
        Ok::<_, String>((module, supported))
    }));
    let (module, supported) = match loaded {
        Ok(Ok(loaded)) => loaded,
        Ok(Err(reason)) => return load_failed(&shared, &stream, reason),
        Err(payload) => return load_failed(&shared, &stream, format!("panicked: {}", panic_message(&*payload))),
    };
    shared.send(&ChildMessage::Loaded {
        module_type: shared.category.clone(),
        supports_eye: supported.eye,
        supports_expression: supported.expression,
    });

    let code = read_loop(&shared, &stream, module, options.data_dir);
    close(&stream);
    if shared.crashed.load(Ordering::SeqCst) { exit::CRASHED } else { code }
}

fn load_failed(shared: &Shared, stream: &TcpStream, reason: String) -> i32 {
    shared.log(LogLevel::Error, format!("Module load failed: {reason}"));
    shared.send(&ChildMessage::LoadFailed { reason });
    close(stream);
    exit::LOAD_FAILED
}

/// Handles OSCFT's messages until `Teardown` or the connection ends.
fn read_loop<M: Module>(shared: &Arc<Shared>, stream: &TcpStream, module: M, data_dir: PathBuf) -> i32 {
    let (event_tx, event_rx) = mpsc::channel();
    let mut pending = Some((module, event_rx));
    let mut worker: Option<mpsc::Receiver<()>> = None;
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        let message = match read_host_message(&mut reader, &mut buf) {
            Ok(m) => m,
            Err(e) => {
                if !shared.crashed.load(Ordering::SeqCst) && !matches!(e, ProtocolError::Closed) {
                    eprintln!("connection to OSCFT failed: {e}");
                }
                stop_worker(shared, worker);
                return if matches!(e, ProtocolError::Io(_) | ProtocolError::Closed) { exit::OK } else { exit::CONNECT_FAILED };
            }
        };
        match message {
            HostMessage::Settings { json } => match serde_json::from_str(&json) {
                Ok(Value::Object(values)) => {
                    *shared.settings() = values;
                    // Before Init the values are simply there when `initialize` runs.
                    if worker.is_some() {
                        let _ = event_tx.send(Event::SettingsChanged);
                    }
                }
                _ => shared.log(LogLevel::Warning, "Ignoring malformed settings from OSCFT".into()),
            },
            HostMessage::SettingsAction { key } => {
                let _ = event_tx.send(Event::Action(key));
            }
            HostMessage::Init { eye_available, expression_available } => {
                if let Some((module, events)) = pending.take() {
                    let available = Claims { eye: eye_available, expression: expression_available };
                    let ctx = Context {
                        shared: shared.clone(),
                        events,
                        data_dir: data_dir.clone(),
                        flags: Cell::new(FrameFlags::default()),
                        seq: Cell::new(0),
                        last_frame: Cell::new(None),
                        buf: RefCell::new(Vec::with_capacity(512)),
                        status: Cell::new(None),
                    };
                    worker = Some(spawn_worker(module, ctx, available));
                }
            }
            HostMessage::Teardown => {
                stop_worker(shared, worker);
                shared.send(&ChildMessage::TeardownDone);
                return exit::OK;
            }
        }
    }
}

/// Runs `initialize`, `run` and (after a stop request) `teardown` on the module thread. The returned receiver
/// disconnects or gets `()` once the thread is done.
fn spawn_worker<M: Module>(mut module: M, ctx: Context, available: Claims) -> mpsc::Receiver<()> {
    let (done_tx, done_rx) = mpsc::channel();
    let shared = ctx.shared.clone();
    let spawned = std::thread::Builder::new().name("module".into()).spawn(move || {
        let shared = ctx.shared.clone();
        let tracked = panic::catch_unwind(AssertUnwindSafe(|| {
            let init = module.initialize(&ctx, available);
            let claims = init.claims.intersect(available);
            ctx.flags.set(claims.flags());
            shared.send(&ChildMessage::InitResult {
                eye: claims.eye,
                expression: claims.expression,
                name: init.name,
                images: init.images,
            });
            if (claims.eye || claims.expression) && !ctx.is_stopping() {
                module.run(&ctx);
            }
        }));
        // A panic while being stopped is the module reacting to the stop; anything else is a crash.
        if let Err(payload) = tracked {
            if !shared.is_stopping() {
                shared.log(LogLevel::Critical, format!("Module panicked: {}", panic_message(&*payload)));
                shared.crash();
                return;
            }
        }
        shared.wait_stopping(None);
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| module.teardown(&ctx))) {
            shared.log(LogLevel::Error, format!("Module teardown panicked: {}", panic_message(&*payload)));
        }
        let _ = done_tx.send(());
    });
    if let Err(e) = spawned {
        shared.log(LogLevel::Critical, format!("Cannot start the module thread: {e}"));
        shared.crash();
    }
    done_rx
}

/// Asks the module to stop and waits for its `teardown` to finish.
fn stop_worker(shared: &Shared, worker: Option<mpsc::Receiver<()>>) {
    shared.stop();
    if let Some(done) = worker {
        if let Err(mpsc::RecvTimeoutError::Timeout) = done.recv_timeout(RUN_STOP_TIMEOUT) {
            shared.log(
                LogLevel::Warning,
                format!("run() did not return within {} s of the stop request", RUN_STOP_TIMEOUT.as_secs()),
            );
        }
    }
}

/// Closes the connection gracefully: OSCFT reads every message sent so far instead of seeing a reset.
fn close(stream: &TcpStream) {
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(CLOSE_TIMEOUT));
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    let mut scratch = [0u8; 256];
    let mut s = stream;
    while Instant::now() < deadline {
        match s.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests;
