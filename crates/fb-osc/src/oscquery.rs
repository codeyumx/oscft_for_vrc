//! OSCQuery: the HTTP server describing FaceBridge's OSC endpoint, mDNS advertisement/discovery and the small
//! HTTP client used to read VRChat's `/avatar` parameter tree.
//!
//! Specs: <https://github.com/Vidvox/OSCQueryProposal>,
//! <https://github.com/vrchat-community/osc/wiki/OSCQuery>.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
use serde_json::{Value as Json, json};

const SERVICE_NAME: &str = "FaceBridge";
const OSCJSON_TYPE: &str = "_oscjson._tcp.local.";
const OSC_TYPE: &str = "_osc._udp.local.";

// ---- HTTP server ---------------------------------------------------------------------------------------------------

/// What FaceBridge exposes: `/avatar/change` (string, write) so VRChat sends it avatar changes.
fn tree() -> Json {
    json!({
        "DESCRIPTION": "root node",
        "FULL_PATH": "/",
        "ACCESS": 0,
        "CONTENTS": {
            "avatar": {
                "DESCRIPTION": "avatar",
                "FULL_PATH": "/avatar",
                "ACCESS": 2,
                "CONTENTS": {
                    "change": {
                        "DESCRIPTION": "avatar id of the loaded avatar",
                        "FULL_PATH": "/avatar/change",
                        "ACCESS": 2,
                        "TYPE": "s"
                    }
                }
            }
        }
    })
}

fn host_info(osc_ip: Option<IpAddr>, osc_port: u16) -> Json {
    let mut info = json!({
        "NAME": SERVICE_NAME,
        "EXTENSIONS": {
            "ACCESS": true,
            "VALUE": false,
            "RANGE": false,
            "DESCRIPTION": true,
            "TAGS": false,
            "CLIPMODE": false,
            "UNIT": false,
        },
        "OSC_PORT": osc_port,
        "OSC_TRANSPORT": "UDP",
    });
    // Without OSC_IP clients use the OSCQuery server's own address.
    if let Some(ip) = osc_ip {
        info["OSC_IP"] = ip.to_string().into();
    }
    info
}

/// Answer one OSCQuery request. `Ok(None)` = 204 (known node, attribute without value), `Err(code)` = error.
fn answer(url: &str, tree: &Json, host_info: &Json) -> Result<Option<Json>, u16> {
    let url = url.split('#').next().unwrap_or(url); // fragments are discarded
    let (path, query) = url.split_once('?').map_or((url, ""), |(p, q)| (p, q));
    if query == "HOST_INFO" {
        return Ok(Some(host_info.clone()));
    }
    let mut node = tree;
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        node = node.get("CONTENTS").and_then(|c| c.get(seg)).ok_or(404u16)?;
    }
    if query.is_empty() {
        return Ok(Some(node.clone()));
    }
    match node.get(query) {
        Some(v) => Ok(Some(json!({ query: v }))),
        None if matches!(query, "VALUE" | "RANGE" | "UNIT" | "TAGS" | "CLIPMODE" | "EXTENDED_TYPE") => Ok(None),
        None => Err(400),
    }
}

/// The OSCQuery HTTP server. Stops when dropped.
pub struct HttpServer {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Bind `bind_ip:0` and serve the tree; `osc_ip` (when known) and `osc_port` are reported in HOST_INFO.
    pub fn start(bind_ip: IpAddr, osc_ip: Option<IpAddr>, osc_port: u16) -> io::Result<Self> {
        let server = tiny_http::Server::http(SocketAddr::new(bind_ip, 0)).map_err(io::Error::other)?;
        let port = server.server_addr().to_ip().map(|a| a.port()).ok_or_else(|| io::Error::other("no TCP address"))?;
        let tree = tree();
        let info = host_info(osc_ip, osc_port);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let json_header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("static header");
        let thread = std::thread::Builder::new().name("fb-osc-http".into()).spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                let request = match server.recv_timeout(Duration::from_millis(200)) {
                    Ok(Some(r)) => r,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                let _ = match answer(request.url(), &tree, &info) {
                    Ok(Some(body)) => request.respond(tiny_http::Response::from_string(body.to_string()).with_header(json_header.clone())),
                    Ok(None) => request.respond(tiny_http::Response::empty(204)),
                    Err(code) => request.respond(tiny_http::Response::empty(code)),
                };
            }
        })?;
        Ok(Self { port, stop, thread: Some(thread) })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ---- HTTP client ---------------------------------------------------------------------------------------------------

const MAX_BODY: u64 = 32 * 1024 * 1024;

/// Read one HTTP/1.1 response: status code and (de-chunked) body.
fn read_response<R: BufRead>(r: &mut R) -> io::Result<(u16, Vec<u8>)> {
    let bad = |m: &'static str| io::Error::new(io::ErrorKind::InvalidData, m);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let status: u16 = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad HTTP status line"))?;
    let mut length: Option<u64> = None;
    let mut chunked = false;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Err(bad("truncated HTTP headers"));
        }
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim(), v.trim());
            if k.eq_ignore_ascii_case("content-length") {
                length = v.parse().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            r.read_line(&mut line)?;
            let size = line.trim().split(';').next().unwrap_or("");
            let size = u64::from_str_radix(size, 16).map_err(|_| bad("bad chunk size"))?;
            if size == 0 {
                break;
            }
            if body.len() as u64 + size > MAX_BODY {
                return Err(bad("response too large"));
            }
            let start = body.len();
            body.resize(start + size as usize, 0);
            r.read_exact(&mut body[start..])?;
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf)?;
        }
    } else if let Some(n) = length {
        if n > MAX_BODY {
            return Err(bad("response too large"));
        }
        body.resize(n as usize, 0);
        r.read_exact(&mut body)?;
    } else {
        r.take(MAX_BODY).read_to_end(&mut body)?;
    }
    Ok((status, body))
}

/// `GET path` on an OSCQuery server and parse the JSON body.
pub fn get_json(addr: SocketAddr, path: &str, timeout: Duration) -> io::Result<Json> {
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nConnection: close\r\n\r\n")?;
    let (status, body) = read_response(&mut BufReader::new(stream))?;
    if status != 200 {
        return Err(io::Error::other(format!("OSCQuery {addr}{path}: HTTP {status}")));
    }
    serde_json::from_slice(&body).map_err(io::Error::other)
}

// ---- mDNS ----------------------------------------------------------------------------------------------------------

/// A VRChat OSCQuery service seen on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VrchatService {
    pub fullname: String,
    pub addrs: Vec<IpAddr>,
    pub port: u16,
}

impl VrchatService {
    /// HTTP endpoints to try, best first. VRChat on Windows only serves OSCQuery to the local machine, so loopback
    /// comes first when `local` (FaceBridge sends to this machine); otherwise only the advertised addresses.
    pub fn candidates(&self, local: bool) -> Vec<SocketAddr> {
        let mut v = Vec::new();
        if local {
            v.push(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), self.port));
        }
        let mut addrs: Vec<IpAddr> = self.addrs.iter().copied().filter(|a| !(local && a.is_loopback())).collect();
        addrs.sort_by_key(|a| (a.is_ipv6(), *a));
        v.extend(addrs.into_iter().map(|a| SocketAddr::new(a, self.port)));
        v
    }
}

pub enum DiscoveryEvent {
    Found(VrchatService),
    Removed(String),
}

fn is_vrchat(fullname: &str) -> bool {
    fullname.to_ascii_lowercase().starts_with("vrchat-client")
}

/// mDNS daemon: advertises FaceBridge's OSC receiver and browses for VRChat.
pub struct Mdns {
    daemon: ServiceDaemon,
    browse: mdns_sd::Receiver<ServiceEvent>,
    registered: Vec<String>,
}

fn mdns_err(e: mdns_sd::Error) -> io::Error {
    io::Error::other(format!("mDNS: {e}"))
}

impl Mdns {
    pub fn start() -> io::Result<Self> {
        let daemon = ServiceDaemon::new().map_err(mdns_err)?;
        // VRChat (and our own advertisement to it) also uses the loopback interface on a single machine.
        daemon.enable_interface(IfKind::LoopbackV4).map_err(mdns_err)?;
        let browse = daemon.browse(OSCJSON_TYPE).map_err(mdns_err)?;
        Ok(Self { daemon, browse, registered: Vec::new() })
    }

    /// Advertise `_osc._udp` (`udp_port`) and `_oscjson._tcp` (`http_port`). With `local_only` the service is
    /// announced on 127.0.0.1 only; otherwise on every address of this host.
    pub fn advertise(&mut self, udp_port: u16, http_port: u16, local_only: bool) -> io::Result<()> {
        let host = format!("facebridge-{:x}.local.", std::process::id());
        for (ty, port) in [(OSC_TYPE, udp_port), (OSCJSON_TYPE, http_port)] {
            let props = None::<std::collections::HashMap<String, String>>;
            let mut info = if local_only {
                ServiceInfo::new(ty, SERVICE_NAME, &host, "127.0.0.1", port, props).map_err(mdns_err)?
            } else {
                ServiceInfo::new(ty, SERVICE_NAME, &host, "", port, props).map_err(mdns_err)?.enable_addr_auto()
            };
            info.set_requires_probe(false);
            self.registered.push(info.get_fullname().to_owned());
            self.daemon.register(info).map_err(mdns_err)?;
        }
        Ok(())
    }

    /// Drain pending browse events, keeping only VRChat's service.
    pub fn poll(&self) -> Vec<DiscoveryEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.browse.try_recv() {
            match event {
                ServiceEvent::ServiceResolved(info) if is_vrchat(info.get_fullname()) => {
                    out.push(DiscoveryEvent::Found(VrchatService {
                        fullname: info.get_fullname().to_owned(),
                        addrs: info.get_addresses().iter().copied().collect(),
                        port: info.get_port(),
                    }));
                }
                ServiceEvent::ServiceRemoved(_, fullname) if is_vrchat(&fullname) => out.push(DiscoveryEvent::Removed(fullname)),
                _ => {}
            }
        }
        out
    }

    /// Withdraw the advertisement (goodbye packets) and stop the daemon.
    pub fn shutdown(self) {
        for name in &self.registered {
            if let Ok(rx) = self.daemon.unregister(name) {
                let _ = rx.recv_timeout(Duration::from_millis(300));
            }
        }
        if let Ok(rx) = self.daemon.shutdown() {
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn answers_host_info_tree_and_attributes() {
        let t = tree();
        let h = host_info(Some(Ipv4Addr::LOCALHOST.into()), 9001);
        let info = answer("/?HOST_INFO", &t, &h).unwrap().unwrap();
        assert_eq!(info["OSC_PORT"], 9001);
        assert_eq!(info["OSC_IP"], "127.0.0.1");
        assert_eq!(info["OSC_TRANSPORT"], "UDP");

        let root = answer("/", &t, &h).unwrap().unwrap();
        assert_eq!(root["CONTENTS"]["avatar"]["CONTENTS"]["change"]["TYPE"], "s");
        let avatar = answer("/avatar", &t, &h).unwrap().unwrap();
        assert_eq!(avatar["FULL_PATH"], "/avatar");
        assert_eq!(answer("/avatar/change?TYPE", &t, &h).unwrap().unwrap(), json!({"TYPE": "s"}));
        assert_eq!(answer("/avatar/change#frag", &t, &h).unwrap().unwrap()["FULL_PATH"], "/avatar/change");
        assert_eq!(answer("/avatar/change?VALUE", &t, &h), Ok(None));
        assert_eq!(answer("/avatar/change?BOGUS", &t, &h), Err(400));
        assert_eq!(answer("/nope", &t, &h), Err(404));
    }

    #[test]
    fn parses_content_length_and_chunked_responses() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 7\r\n\r\n{\"a\":1}";
        let (s, b) = read_response(&mut Cursor::new(&plain[..])).unwrap();
        assert_eq!((s, b.as_slice()), (200, &b"{\"a\":1}"[..]));

        let chunked = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3;ext=1\r\n:1}\r\n0\r\n\r\n";
        let (s, b) = read_response(&mut Cursor::new(&chunked[..])).unwrap();
        assert_eq!((s, b.as_slice()), (200, &b"{\"a\":1}"[..]));

        let (s, b) = read_response(&mut Cursor::new(&b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n"[..])).unwrap();
        assert_eq!((s, b.len()), (404, 0));
        assert!(read_response(&mut Cursor::new(&b"garbage"[..])).is_err());
    }

    #[test]
    fn http_server_round_trip() {
        let server = HttpServer::start(Ipv4Addr::LOCALHOST.into(), Some(Ipv4Addr::LOCALHOST.into()), 4242).unwrap();
        let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), server.port());
        let t = Duration::from_secs(3);
        assert_eq!(get_json(addr, "/?HOST_INFO", t).unwrap()["OSC_PORT"], 4242);
        assert_eq!(get_json(addr, "/avatar", t).unwrap()["FULL_PATH"], "/avatar");
        assert!(get_json(addr, "/missing", t).is_err());
    }

    #[test]
    fn candidate_order_prefers_loopback_when_local() {
        let svc = VrchatService {
            fullname: "VRChat-Client-ABC123._oscjson._tcp.local.".into(),
            addrs: vec!["192.168.1.5".parse().unwrap(), "127.0.0.1".parse().unwrap(), "fe80::1".parse().unwrap()],
            port: 9999,
        };
        let local: Vec<String> = svc.candidates(true).iter().map(ToString::to_string).collect();
        assert_eq!(local, ["127.0.0.1:9999", "192.168.1.5:9999", "[fe80::1]:9999"]);
        let remote: Vec<String> = svc.candidates(false).iter().map(ToString::to_string).collect();
        assert_eq!(remote, ["127.0.0.1:9999", "192.168.1.5:9999", "[fe80::1]:9999"]);
        assert!(is_vrchat(&svc.fullname));
        assert!(!is_vrchat("FaceBridge._oscjson._tcp.local."));
    }
}
