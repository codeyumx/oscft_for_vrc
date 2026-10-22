//! Threads behind [`crate::OscOutput`]: sender, legacy `/avatar/change` listener and avatar discovery.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

use fb_tracking::TrackingFrame;

use crate::avatar::{self, AvatarParams};
use crate::lock;
use crate::osc::{self, Bundle};
use crate::oscquery::{self, DiscoveryEvent, HttpServer, Mdns, VrchatService};
use crate::params::{self, SendPlan};
use crate::{AvatarInfo, AvatarSource, Claims, OscConfig, OscEvent, OscStatus};

/// Largest datagram we build (stays below a typical 1500 byte MTU).
const MAX_DATAGRAM: usize = 1400;
/// Everything is sent again at least this often, so values VRChat dropped (avatar still loading, VRChat
/// restarted, packet loss) recover without waiting for the tracking data to move.
const RESEND_INTERVAL: Duration = Duration::from_secs(3);
/// VRChat is considered present for this long after the last datagram it sent us.
const RX_WINDOW: Duration = Duration::from_secs(15);
/// How often VRChat's OSCQuery service is asked which avatar is loaded (catches missed `/avatar/change`).
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Retry schedule for resolving a new avatar's parameters (the config file and tree appear shortly after load).
const RESOLVE_DELAYS_MS: [u64; 8] = [0, 250, 500, 1000, 2000, 3000, 5000, 8000];
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct Latest {
    pub frame: TrackingFrame,
    pub claims: Claims,
    /// Number of submitted frames (0 = nothing submitted yet).
    pub seq: u64,
}

pub(crate) struct Inner {
    pub status: OscStatus,
    pub avatar: Option<AvatarInfo>,
    send_err: Option<String>,
    avatar_note: Option<String>,
    listener_note: Option<String>,
    vrchat_mdns: bool,
    rx_recent: bool,
}

pub(crate) struct Shared {
    pub config: OscConfig,
    pub stop: AtomicBool,
    pub latest: Mutex<Latest>,
    pub inner: Mutex<Inner>,
    on_event: Box<dyn Fn(OscEvent) + Send + Sync>,
    /// Plan handed to the sender thread: `Some(None)` = clear, `Some(Some(plan))` = replace.
    pending_plan: Mutex<Option<Option<SendPlan>>>,
    target: Mutex<SocketAddr>,
    /// Milliseconds since `epoch` of the last datagram received (0 = never).
    last_rx_ms: AtomicU64,
    epoch: Instant,
}

impl Shared {
    /// Mutate the shared state; emits `StatusChanged` (outside the lock) when the public status changed.
    pub fn update(&self, f: impl FnOnce(&mut Inner)) {
        let changed = {
            let mut inner = lock(&self.inner);
            let before = inner.status.clone();
            f(&mut inner);
            inner.status.last_error = inner.send_err.clone().or_else(|| inner.avatar_note.clone()).or_else(|| inner.listener_note.clone());
            inner.status.vrchat_detected = inner.vrchat_mdns || inner.rx_recent;
            (inner.status != before).then(|| inner.status.clone())
        };
        if let Some(s) = changed {
            (self.on_event)(OscEvent::StatusChanged(s));
        }
    }

    /// Publish the avatar and the plan for it (`None`/`None` when the avatar is gone).
    fn set_avatar(&self, info: Option<AvatarInfo>, plan: Option<SendPlan>) {
        *lock(&self.pending_plan) = Some(plan);
        lock(&self.inner).avatar = info.clone();
        (self.on_event)(OscEvent::AvatarChanged(info));
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + 1
    }
}

pub(crate) enum DiscoveryMsg {
    AvatarChange(String),
}

type Started = (Arc<Shared>, Vec<JoinHandle<()>>, Thread, mpsc::Sender<DiscoveryMsg>);

pub(crate) fn start(mut config: OscConfig, on_event: Box<dyn Fn(OscEvent) + Send + Sync>) -> io::Result<Started> {
    config.rate_hz = config.rate_hz.clamp(1, 240);
    let local = config.send_addr.ip().is_loopback();

    let bind_addr: SocketAddr = match (config.send_addr, local) {
        (SocketAddr::V4(_), true) => (Ipv4Addr::LOCALHOST, 0).into(),
        (SocketAddr::V6(_), true) => (std::net::Ipv6Addr::LOCALHOST, 0).into(),
        (SocketAddr::V4(_), false) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        (SocketAddr::V6(_), false) => (std::net::Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let send_socket = UdpSocket::bind(bind_addr)?;

    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        latest: Mutex::new(Latest { frame: TrackingFrame::default(), claims: Claims::default(), seq: 0 }),
        inner: Mutex::new(Inner {
            status: OscStatus { send_addr: config.send_addr, sending: false, messages_per_second: 0, last_error: None, vrchat_detected: false },
            avatar: None,
            send_err: None,
            avatar_note: None,
            listener_note: None,
            vrchat_mdns: false,
            rx_recent: false,
        }),
        on_event,
        pending_plan: Mutex::new(None),
        target: Mutex::new(config.send_addr),
        last_rx_ms: AtomicU64::new(0),
        epoch: Instant::now(),
        config,
    });
    let config = &shared.config;

    // Legacy listener: the configured port, else (port busy / OSCQuery-only) an ephemeral one that only
    // OSCQuery advertises.
    let listen_ip: IpAddr = if local { Ipv4Addr::LOCALHOST.into() } else { Ipv4Addr::UNSPECIFIED.into() };
    let mut notes: Vec<String> = Vec::new();
    let mut listener: Option<UdpSocket> = None;
    if config.listen_port != 0 {
        match UdpSocket::bind((listen_ip, config.listen_port)) {
            Ok(s) => listener = Some(s),
            Err(e) if config.oscquery => notes.push(format!("OSC listen port {} unavailable ({e}); using OSCQuery only", config.listen_port)),
            Err(e) => notes.push(format!("OSC listen port {} unavailable ({e}); avatar changes cannot be detected", config.listen_port)),
        }
    }
    if listener.is_none() && config.oscquery {
        match UdpSocket::bind((listen_ip, 0)) {
            Ok(s) => listener = Some(s),
            Err(e) => notes.push(format!("cannot open an OSC receive port: {e}")),
        }
    }
    let udp_port = listener.as_ref().and_then(|s| s.local_addr().ok()).map(|a| a.port());

    // OSCQuery: HTTP server + mDNS advertisement + browse for VRChat.
    let mut http: Option<HttpServer> = None;
    let mut mdns: Option<Mdns> = None;
    if config.oscquery {
        if let Some(udp) = udp_port {
            // HOST_INFO tells VRChat where to send UDP; for a remote VRChat the address is left out ("same host").
            match HttpServer::start(listen_ip, local.then_some(IpAddr::from(Ipv4Addr::LOCALHOST)), udp) {
                Ok(s) => http = Some(s),
                Err(e) => notes.push(format!("OSCQuery HTTP server failed: {e}")),
            }
        }
        match Mdns::start() {
            Ok(mut m) => {
                if let (Some(udp), Some(h)) = (udp_port, http.as_ref()) {
                    if let Err(e) = m.advertise(udp, h.port(), local) {
                        notes.push(format!("OSCQuery advertisement failed: {e}"));
                    }
                }
                mdns = Some(m);
            }
            Err(e) => notes.push(format!("OSCQuery discovery unavailable: {e}")),
        }
    }
    if !notes.is_empty() {
        let note = notes.join("; ");
        shared.update(|i| i.listener_note = Some(note));
    }

    let (tx, rx) = mpsc::channel();
    let mut threads = Vec::new();

    let sender_handle = {
        let shared = Arc::clone(&shared);
        thread::Builder::new().name("fb-osc-send".into()).spawn(move || sender_loop(&shared, &send_socket))?
    };
    let sender = sender_handle.thread().clone();
    threads.push(sender_handle);

    if let Some(socket) = listener {
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        let shared = Arc::clone(&shared);
        let tx = tx.clone();
        threads.push(thread::Builder::new().name("fb-osc-recv".into()).spawn(move || listener_loop(&shared, &socket, &tx))?);
    }

    if config.oscquery || udp_port.is_some() {
        let shared2 = Arc::clone(&shared);
        let mut disc = Discovery::new(shared2, rx, mdns, local);
        threads.push(thread::Builder::new().name("fb-osc-discovery".into()).spawn(move || {
            disc.run();
            // The HTTP server lives exactly as long as the discovery thread.
            drop(http);
        })?);
    } else {
        drop(mdns);
        drop(http);
    }

    Ok((shared, threads, sender, tx))
}


// ---- sender ----------------------------------------------------------------------------------------------------

fn flush(socket: &UdpSocket, bundle: &mut Bundle, target: SocketAddr, sent: &mut u32, err: &mut Option<String>, attempted: &mut bool) {
    if let Some(packet) = bundle.packet() {
        *attempted = true;
        match socket.send_to(packet, target) {
            Ok(_) => *sent += bundle.len() as u32,
            Err(e) => *err = Some(format!("send to {target} failed: {e}")),
        }
    }
    bundle.clear();
}

fn sender_loop(shared: &Shared, socket: &UdpSocket) {
    let period = Duration::from_secs_f64(1.0 / f64::from(shared.config.rate_hz));
    let mut plan: Option<SendPlan> = None;
    let mut bundle = Bundle::new(MAX_DATAGRAM);
    let mut next = Instant::now();
    let mut last_full = next;
    let mut force_full = true;
    let mut window_start = next;
    let mut rates = [0u32; 3];
    let mut cur_err: Option<String> = None;

    while !shared.stop.load(Ordering::Relaxed) {
        next += period;
        loop {
            let now = Instant::now();
            if now >= next || shared.stop.load(Ordering::Relaxed) {
                if now > next + period * 4 {
                    next = now; // fell behind (suspend, long stall): do not burst to catch up
                }
                break;
            }
            thread::park_timeout(next - now);
        }
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }

        if let Some(p) = lock(&shared.pending_plan).take() {
            plan = p;
            force_full = true;
        }
        let (frame, claims, seq) = {
            let l = lock(&shared.latest);
            (l.frame, l.claims, l.seq)
        };
        let now = Instant::now();
        if now.duration_since(last_full) >= RESEND_INTERVAL {
            force_full = true;
        }

        if let Some(plan) = plan.as_mut() {
            let target = *lock(&shared.target);
            let (mut sent, mut err, mut attempted) = (0u32, None, false);
            plan.collect(&frame, claims, force_full, &mut |address, value| {
                if !bundle.push(address, value) {
                    flush(socket, &mut bundle, target, &mut sent, &mut err, &mut attempted);
                    bundle.push(address, value);
                }
            });
            flush(socket, &mut bundle, target, &mut sent, &mut err, &mut attempted);
            rates[0] += sent;
            if attempted && cur_err != err {
                cur_err = err.clone();
                shared.update(|i| i.send_err = err);
            }
        }
        if force_full {
            force_full = false;
            last_full = now;
        }

        let elapsed = now.duration_since(window_start);
        if elapsed >= Duration::from_secs(1) {
            // Messages per second averaged over the last three one-second windows, so the periodic full resend
            // does not make the figure flap between 0 and N.
            let mps = (f64::from(rates.iter().sum::<u32>()) / 3.0).round() as u32;
            let sending = plan.as_ref().is_some_and(|p| !p.is_empty()) && seq > 0 && cur_err.is_none();
            let last_rx = shared.last_rx_ms.load(Ordering::Relaxed);
            let rx_recent = last_rx != 0 && Duration::from_millis(shared.now_ms().saturating_sub(last_rx)) < RX_WINDOW;
            shared.update(|i| {
                i.status.messages_per_second = mps;
                i.status.sending = sending;
                i.rx_recent = rx_recent;
            });
            window_start = now;
            rates.rotate_right(1);
            rates[0] = 0;
        }
    }
}

// ---- legacy listener ---------------------------------------------------------------------------------------------

fn listener_loop(shared: &Shared, socket: &UdpSocket, tx: &mpsc::Sender<DiscoveryMsg>) {
    let mut buf = vec![0u8; 65536];
    while !shared.stop.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buf) {
            Ok((n, _)) => {
                shared.last_rx_ms.store(shared.now_ms(), Ordering::Relaxed);
                let data = &buf[..n];
                // VRChat streams every avatar parameter change here; only decode what can hold /avatar/change.
                if data.starts_with(b"/avatar/change\0") || data.starts_with(b"#bundle\0") {
                    if let Some(id) = osc::avatar_change(data) {
                        let _ = tx.send(DiscoveryMsg::AvatarChange(id));
                    }
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::ConnectionReset) => {}
            Err(e) => {
                shared.update(|i| i.listener_note = Some(format!("OSC receive failed: {e}")));
                break;
            }
        }
    }
}

// ---- discovery ---------------------------------------------------------------------------------------------------

struct Pending {
    id: String,
    attempt: usize,
    due: Instant,
}

struct Discovery {
    shared: Arc<Shared>,
    rx: mpsc::Receiver<DiscoveryMsg>,
    mdns: Option<Mdns>,
    vrchat: Option<VrchatService>,
    local: bool,
    config_root: Option<PathBuf>,
    current_id: Option<String>,
    pending: Option<Pending>,
    last_poll: Instant,
    poll_now: bool,
    target_checked: bool,
}

impl Discovery {
    fn new(shared: Arc<Shared>, rx: mpsc::Receiver<DiscoveryMsg>, mdns: Option<Mdns>, local: bool) -> Self {
        Self {
            shared,
            rx,
            mdns,
            vrchat: None,
            local,
            config_root: avatar::config_root(),
            current_id: None,
            pending: None,
            last_poll: Instant::now(),
            poll_now: false,
            target_checked: false,
        }
    }

    fn run(&mut self) {
        while !self.shared.stop.load(Ordering::Relaxed) {
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(DiscoveryMsg::AvatarChange(id)) => self.avatar_changed(id),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.poll_mdns();
            self.step_pending();
            self.poll_vrchat();
        }
        if let Some(m) = self.mdns.take() {
            m.shutdown();
        }
    }

    fn poll_mdns(&mut self) {
        let Some(mdns) = &self.mdns else { return };
        for event in mdns.poll() {
            match event {
                DiscoveryEvent::Found(svc) => {
                    if self.vrchat.as_ref() != Some(&svc) {
                        self.target_checked = false;
                        self.poll_now = true;
                        self.vrchat = Some(svc);
                        self.shared.update(|i| i.vrchat_mdns = true);
                    }
                }
                DiscoveryEvent::Removed(name) => {
                    if self.vrchat.as_ref().is_some_and(|s| s.fullname == name) {
                        self.vrchat = None;
                        self.shared.update(|i| i.vrchat_mdns = false);
                    }
                }
            }
        }
    }

    fn query(&self, path: &str) -> Option<serde_json::Value> {
        let svc = self.vrchat.as_ref()?;
        svc.candidates(self.local).into_iter().find_map(|addr| oscquery::get_json(addr, path, HTTP_TIMEOUT).ok())
    }

    fn fetch_tree(&self) -> Option<AvatarParams> {
        self.query("/avatar").or_else(|| self.query("/")).and_then(|t| avatar::parse_oscquery_tree(&t))
    }

    /// If FaceBridge still targets the default address and VRChat reports a different OSC port (it picks a free
    /// one when 9000 is taken), follow VRChat.
    fn check_target(&mut self) {
        if !self.local || self.shared.config.send_addr != OscConfig::DEFAULT_SEND_ADDR {
            self.target_checked = true;
            return;
        }
        let Some(info) = self.query("/?HOST_INFO") else { return };
        self.target_checked = true;
        let Some(port) = info.get("OSC_PORT").and_then(serde_json::Value::as_u64).and_then(|p| u16::try_from(p).ok()) else {
            return;
        };
        let mut target = lock(&self.shared.target);
        if target.port() != port && port != 0 {
            target.set_port(port);
            let addr = *target;
            drop(target);
            self.shared.update(|i| i.status.send_addr = addr);
        }
    }

    fn poll_vrchat(&mut self) {
        if self.vrchat.is_none() || self.pending.is_some() {
            return;
        }
        if !self.poll_now && self.last_poll.elapsed() < POLL_INTERVAL {
            return;
        }
        self.poll_now = false;
        self.last_poll = Instant::now();
        if !self.target_checked {
            self.check_target();
        }
        let id = self
            .query("/avatar/change")
            .and_then(|n| n.get("VALUE").and_then(|v| v.get(0)).and_then(|v| v.as_str().map(str::to_owned)))
            .filter(|id| !id.is_empty());
        if let Some(id) = id {
            if self.current_id.as_deref() != Some(&id) {
                self.avatar_changed(id);
            }
        }
    }

    fn avatar_changed(&mut self, id: String) {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            return;
        }
        if self.current_id.as_deref() != Some(&id) && self.current_id.take().is_some() {
            // The previous avatar is gone; stop sending its parameters.
            self.shared.set_avatar(None, None);
        }
        self.pending = Some(Pending { id, attempt: 0, due: Instant::now() });
    }

    fn resolve(&self, id: &str) -> Option<(AvatarParams, AvatarSource)> {
        if let Some(tree) = self.fetch_tree() {
            if tree.id == id {
                return Some((tree, AvatarSource::OscQuery));
            }
        }
        let root = self.config_root.as_deref()?;
        match avatar::load_config(root, id) {
            Ok(Some(a)) => Some((a, AvatarSource::ConfigFile)),
            Ok(None) | Err(_) => None,
        }
    }

    fn step_pending(&mut self) {
        let Some(mut p) = self.pending.take() else { return };
        if Instant::now() < p.due {
            self.pending = Some(p);
            return;
        }
        match self.resolve(&p.id) {
            Some((a, source)) => self.install(&p.id, a, source),
            None => {
                p.attempt += 1;
                match RESOLVE_DELAYS_MS.get(p.attempt) {
                    Some(ms) => {
                        p.due = Instant::now() + Duration::from_millis(*ms);
                        self.pending = Some(p);
                    }
                    None => {
                        let note = format!(
                            "no parameter list for avatar {} (OSCQuery tree and VRChat OSC config file unavailable; Build & Test avatars have no config file)",
                            p.id
                        );
                        self.shared.update(|i| i.avatar_note = Some(note));
                    }
                }
            }
        }
    }

    fn install(&mut self, id: &str, a: AvatarParams, source: AvatarSource) {
        let plan = params::build_plan(&a.params);
        let info = AvatarInfo {
            id: id.to_owned(),
            name: a.name,
            parameter_count: a.total.max(a.params.len()),
            matched_parameter_count: plan.len(),
            source,
        };
        self.current_id = Some(id.to_owned());
        self.shared.update(|i| i.avatar_note = None);
        self.shared.set_avatar(Some(info), Some(plan));
    }
}
