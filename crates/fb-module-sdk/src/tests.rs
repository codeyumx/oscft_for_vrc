//! Drives [`serve`] from the OSCFT side of a real loopback connection.

use super::*;
use fb_protocol::{read_child_message, write_host_message};
use std::net::TcpListener;
use std::thread::JoinHandle;

struct Host {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    buf: Vec<u8>,
}

impl Host {
    fn recv(&mut self) -> ChildMessage {
        read_child_message(&mut self.reader, &mut self.buf).expect("child message")
    }

    /// Next message that is not a frame or log line.
    fn recv_control(&mut self) -> ChildMessage {
        loop {
            match self.recv() {
                ChildMessage::Frame(_) | ChildMessage::Log { .. } => {}
                m => return m,
            }
        }
    }

    fn send(&mut self, m: HostMessage) {
        write_host_message(&mut self.stream, &m).unwrap();
    }
}

fn start<M: Module>(load: impl FnOnce(&Path) -> Result<M, String> + Send + 'static) -> (Host, JoinHandle<i32>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let args = [
        "--connect".to_owned(),
        listener.local_addr().unwrap().to_string(),
        "--token".into(),
        "tok".into(),
        "--data-dir".into(),
        std::env::temp_dir().to_string_lossy().into_owned(),
        "--parent-pid".into(),
        "1".into(),
    ];
    let child = std::thread::spawn(move || serve(args.to_vec(), load));
    let (stream, _) = listener.accept().unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut host = Host { reader: BufReader::new(stream.try_clone().unwrap()), stream, buf: Vec::new() };
    match host.recv() {
        ChildMessage::Hello { version, token, .. } => assert_eq!((version, token.as_str()), (PROTOCOL_VERSION, "tok")),
        other => panic!("expected Hello, got {other:?}"),
    }
    (host, child)
}

#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    fn push(&self, s: impl Into<String>) {
        lock(&self.0).push(s.into());
    }

    fn entries(&self) -> Vec<String> {
        lock(&self.0).clone()
    }
}

struct Scripted {
    journal: Journal,
    panic_in_run: bool,
}

impl Module for Scripted {
    fn supported(&self) -> Claims {
        Claims { eye: true, expression: true }
    }

    fn initialize(&mut self, ctx: &Context, available: Claims) -> Init {
        self.journal.push(format!("init speed={}", ctx.settings().get("speed").cloned().unwrap_or_default()));
        ctx.declare_settings(&serde_json::json!({ "fields": [{ "key": "speed", "label": "Speed", "type": "int", "default": 1 }] }));
        Init { name: "Scripted Device".into(), ..Init::new(Claims { eye: false, expression: available.expression }) }
    }

    fn run(&mut self, ctx: &Context) {
        assert!(!self.panic_in_run, "boom");
        let mut frame = TrackingFrame::default();
        frame.set_shape(Expression::JawOpen, 0.5);
        frame.head.yaw = 0.1;
        while ctx.sleep(Duration::from_millis(2)) {
            match ctx.try_event() {
                Some(Event::Action(key)) => ctx.notify(LogLevel::Information, &key, "pressed"),
                Some(Event::SettingsChanged) => self.journal.push(format!("speed={}", ctx.settings()["speed"])),
                None => {}
            }
            ctx.send_frame(&frame);
        }
    }

    fn teardown(&mut self, _ctx: &Context) {
        self.journal.push("teardown");
    }
}

#[test]
fn session_runs_init_frames_events_and_teardown() {
    let journal = Journal::default();
    let module = Scripted { journal: journal.clone(), panic_in_run: false };
    let (mut host, child) = start(move |_| Ok(module));
    match host.recv() {
        ChildMessage::Loaded { module_type, supports_eye, supports_expression } => {
            assert!(module_type.ends_with("Scripted"), "{module_type}");
            assert!(supports_eye && supports_expression);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    host.send(HostMessage::Settings { json: r#"{"speed":2}"#.into() });
    host.send(HostMessage::Init { eye_available: true, expression_available: true });
    assert!(matches!(host.recv(), ChildMessage::SettingsSchema { .. }));
    match host.recv() {
        ChildMessage::InitResult { eye, expression, name, images } => {
            assert_eq!((eye, expression, name.as_str(), images.len()), (false, true, "Scripted Device", 0));
        }
        other => panic!("expected InitResult, got {other:?}"),
    }
    let ChildMessage::Frame(first) = host.recv() else { panic!("expected a frame") };
    assert_eq!(first.seq, 1);
    assert_eq!(first.flags, FrameFlags::EXPRESSION | FrameFlags::HEAD, "eye was not claimed");
    assert_eq!(first.frame.shape(Expression::JawOpen), 0.5);

    host.send(HostMessage::SettingsAction { key: "ping".into() });
    match host.recv_control() {
        ChildMessage::Notify { key, message, .. } => assert_eq!((key.as_str(), message.as_str()), ("ping", "pressed")),
        other => panic!("expected Notify, got {other:?}"),
    }
    host.send(HostMessage::Settings { json: r#"{"speed":3}"#.into() });
    host.send(HostMessage::Teardown);
    assert_eq!(host.recv_control(), ChildMessage::TeardownDone);
    drop(host);
    assert_eq!(child.join().unwrap(), exit::OK);
    let entries = journal.entries();
    assert_eq!(entries.first().map(String::as_str), Some("init speed=2"), "stored settings arrive before initialize");
    assert_eq!(entries.last().map(String::as_str), Some("teardown"));
}

#[test]
fn teardown_before_init_never_initializes_the_module() {
    let journal = Journal::default();
    let module = Scripted { journal: journal.clone(), panic_in_run: false };
    let (mut host, child) = start(move |_| Ok(module));
    assert!(matches!(host.recv(), ChildMessage::Loaded { .. }));
    host.send(HostMessage::Teardown);
    assert_eq!(host.recv_control(), ChildMessage::TeardownDone);
    drop(host);
    assert_eq!(child.join().unwrap(), exit::OK);
    assert!(journal.entries().is_empty(), "{:?}", journal.entries());
}

#[test]
fn panic_in_run_is_logged_and_exits_with_crash_code() {
    let module = Scripted { journal: Journal::default(), panic_in_run: true };
    let (mut host, child) = start(move |_| Ok(module));
    assert!(matches!(host.recv(), ChildMessage::Loaded { .. }));
    host.send(HostMessage::Init { eye_available: false, expression_available: true });
    let crash = loop {
        if let ChildMessage::Log { level: LogLevel::Critical, message, .. } = host.recv() {
            break message;
        }
    };
    assert!(crash.contains("boom"), "{crash}");
    assert!(matches!(read_child_message(&mut host.reader, &mut host.buf), Err(ProtocolError::Closed)));
    drop(host);
    assert_eq!(child.join().unwrap(), exit::CRASHED);
}

#[test]
fn load_error_is_reported_as_load_failed() {
    let (mut host, child) = start(|_| Err::<Scripted, _>("no device driver".into()));
    assert_eq!(host.recv_control(), ChildMessage::LoadFailed { reason: "no device driver".into() });
    drop(host);
    assert_eq!(child.join().unwrap(), exit::LOAD_FAILED);
}

#[test]
fn bad_arguments_exit_without_connecting() {
    let load = |_: &Path| -> Result<Scripted, String> { unreachable!() };
    assert_eq!(serve(vec!["--connect".into()], load), exit::BAD_ARGUMENTS);
    let missing = ["--connect", "127.0.0.1:1", "--token", "t", "--data-dir", "/tmp"].map(String::from).to_vec();
    assert_eq!(serve(missing, load), exit::BAD_ARGUMENTS);
}
