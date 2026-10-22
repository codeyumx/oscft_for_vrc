//! Everything VRChat-facing: OSC encoding, parameter mapping (`spec/osc-output.md`), avatar parameter
//! discovery and sending. No knowledge of modules.

pub mod avatar;
pub mod osc;
pub mod params;
mod oscquery;
mod runtime;

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{JoinHandle, Thread};

use fb_tracking::TrackingFrame;
use serde::{Deserialize, Serialize};

use runtime::{Latest, Shared};

/// Output configuration, persisted by `fb-core`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OscConfig {
    /// Where VRChat listens (default `127.0.0.1:9000`).
    pub send_addr: SocketAddr,
    /// Legacy `/avatar/change` listener port (default 9001); 0 = OSCQuery only.
    pub listen_port: u16,
    /// Output tick rate.
    pub rate_hz: u32,
    /// Advertise and query via OSCQuery.
    pub oscquery: bool,
}

impl OscConfig {
    pub(crate) const DEFAULT_SEND_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9000));
}

impl Default for OscConfig {
    fn default() -> Self {
        Self { send_addr: Self::DEFAULT_SEND_ADDR, listen_port: 9001, rate_hz: 60, oscquery: true }
    }
}

/// Which tracking parts the merged frame actually carries; unclaimed parts are never sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claims {
    pub eye: bool,
    pub expression: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AvatarSource {
    OscQuery,
    ConfigFile,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarInfo {
    pub id: String,
    pub name: Option<String>,
    /// Parameters the avatar declares.
    pub parameter_count: usize,
    /// Avatar parameters FaceBridge sends data to.
    pub matched_parameter_count: usize,
    pub source: AvatarSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OscStatus {
    /// Where messages go right now (VRChat's reported port replaces the default one, see `spec/osc-output.md`).
    pub send_addr: SocketAddr,
    pub sending: bool,
    pub messages_per_second: u32,
    pub last_error: Option<String>,
    pub vrchat_detected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscEvent {
    AvatarChanged(Option<AvatarInfo>),
    StatusChanged(OscStatus),
}

/// The VRChat output pipeline. Owns its sockets and threads; stops on drop.
pub struct OscOutput {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    sender: Thread,
    /// Keeps the discovery thread's command channel open even without a UDP listener.
    _discovery_tx: mpsc::Sender<runtime::DiscoveryMsg>,
}

impl OscOutput {
    /// Start sending to `config.send_addr`. Failures of the optional parts (legacy listener port in use,
    /// OSCQuery) do not fail the start: they are reported through [`OscStatus::last_error`].
    pub fn start(config: OscConfig, on_event: impl Fn(OscEvent) + Send + Sync + 'static) -> io::Result<Self> {
        let (shared, threads, sender, tx) = runtime::start(config, Box::new(on_event))?;
        Ok(Self { shared, threads, sender, _discovery_tx: tx })
    }

    /// Latest frame wins; never blocks on the network.
    pub fn submit(&self, frame: &TrackingFrame, claims: Claims) {
        let mut l = lock(&self.shared.latest);
        *l = Latest { frame: *frame, claims, seq: l.seq + 1 };
    }

    pub fn status(&self) -> OscStatus {
        lock(&self.shared.inner).status.clone()
    }

    pub fn avatar(&self) -> Option<AvatarInfo> {
        lock(&self.shared.inner).avatar.clone()
    }
}

impl Drop for OscOutput {
    fn drop(&mut self) {
        self.shared.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.sender.unpark();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
