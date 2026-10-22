//! End to end: `/avatar/change` on the legacy port -> avatar config file -> send plan -> real OSC datagrams.

use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fb_osc::osc::{self, Arg};
use fb_osc::{AvatarSource, Claims, OscConfig, OscEvent, OscOutput};
use fb_tracking::{Expression, TrackingFrame};

const CONFIG: &str = r#"{
  "id": "avtr_pipeline",
  "name": "Pipeline Avatar",
  "parameters": [
    { "name": "VRCEmote", "input": { "address": "/avatar/parameters/VRCEmote", "type": "Int" } },
    { "name": "FT/v2/JawOpen", "input": { "address": "/avatar/parameters/FT/v2/JawOpen", "type": "Float" } },
    { "name": "FT/v2/JawX", "input": { "address": "/avatar/parameters/FT/v2/JawX", "type": "Float" } },
    { "name": "FT/v2/JawXNegative", "input": { "address": "/avatar/parameters/FT/v2/JawXNegative", "type": "Bool" } },
    { "name": "FT/v2/JawX1", "input": { "address": "/avatar/parameters/FT/v2/JawX1", "type": "Bool" } },
    { "name": "FT/v2/JawX2", "input": { "address": "/avatar/parameters/FT/v2/JawX2", "type": "Bool" } },
    { "name": "FT/v2/EyeLeftX", "input": { "address": "/avatar/parameters/FT/v2/EyeLeftX", "type": "Float" } },
    { "name": "EyeTrackingActive", "input": { "address": "/avatar/parameters/EyeTrackingActive", "type": "Bool" } },
    { "name": "Velocity", "output": { "address": "/avatar/parameters/Velocity", "type": "Float" } }
  ]
}"#;

/// Collect decoded `(address, args)` pairs from `socket` for `window`.
fn drain(socket: &UdpSocket, window: Duration) -> Vec<(String, Vec<Arg>)> {
    let end = Instant::now() + window;
    let mut out = Vec::new();
    let mut buf = [0u8; 2048];
    while Instant::now() < end {
        if let Ok((n, _)) = socket.recv_from(&mut buf) {
            osc::for_each_message(&buf[..n], &mut |a, args| out.push((a, args))).unwrap();
        }
    }
    out
}

fn latest(msgs: &[(String, Vec<Arg>)]) -> HashMap<&str, &Arg> {
    msgs.iter().map(|(a, args)| (a.as_str(), &args[0])).collect()
}

#[test]
fn avatar_change_to_osc_datagrams() {
    let root = std::env::temp_dir().join(format!("fb-osc-pipeline-{}", std::process::id()));
    let dir = root.join("usr_test").join("Avatars");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("avtr_pipeline.json"), CONFIG).unwrap();
    // SAFETY: this is the only test in this binary and no other thread exists yet.
    unsafe { std::env::set_var("FACEBRIDGE_VRCHAT_OSC_DIR", &root) };

    let vrchat = UdpSocket::bind("127.0.0.1:0").unwrap();
    vrchat.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let listen_port = {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.local_addr().unwrap().port()
    };
    let config = OscConfig { send_addr: vrchat.local_addr().unwrap(), listen_port, rate_hz: 60, oscquery: false };
    let (tx, events) = mpsc::channel();
    let out = OscOutput::start(config, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();

    let mut frame = TrackingFrame::default();
    frame.set_shape(Expression::JawOpen, 0.5);
    frame.set_shape(Expression::JawLeft, 5.0 / 3.0); // clamped to 1.0 -> JawX = -1.0
    frame.eye.left.gaze_x = 0.25;
    let claims = Claims { eye: false, expression: true };
    out.submit(&frame, claims);

    // No avatar known yet: nothing is sent.
    assert!(drain(&vrchat, Duration::from_millis(300)).is_empty());

    // VRChat announces the avatar on the legacy port.
    let mut change = Vec::new();
    osc::write_string_message(&mut change, "/avatar/change", "avtr_pipeline");
    UdpSocket::bind("127.0.0.1:0").unwrap().send_to(&change, ("127.0.0.1", listen_port)).unwrap();

    let info = loop {
        if let OscEvent::AvatarChanged(Some(info)) = events.recv_timeout(Duration::from_secs(5)).expect("avatar event") {
            break info;
        }
    };
    assert_eq!(info.id, "avtr_pipeline");
    assert_eq!(info.name.as_deref(), Some("Pipeline Avatar"));
    assert_eq!(info.source, AvatarSource::ConfigFile);
    assert_eq!(info.parameter_count, 9);
    // JawOpen, JawX, Negative, 2 bits, EyeLeftX (matched but unclaimed), EyeTrackingActive
    assert_eq!(info.matched_parameter_count, 7);
    assert_eq!(out.avatar().unwrap().id, "avtr_pipeline");

    let first = drain(&vrchat, Duration::from_millis(400));
    let m = latest(&first);
    assert_eq!(m["/avatar/parameters/FT/v2/JawOpen"], &Arg::Float(0.5));
    assert_eq!(m["/avatar/parameters/FT/v2/JawX"], &Arg::Float(-1.0));
    assert_eq!(m["/avatar/parameters/FT/v2/JawXNegative"], &Arg::Bool(true));
    assert_eq!(m["/avatar/parameters/FT/v2/JawX1"], &Arg::Bool(true));
    assert_eq!(m["/avatar/parameters/FT/v2/JawX2"], &Arg::Bool(true));
    assert_eq!(m["/avatar/parameters/EyeTrackingActive"], &Arg::Bool(false));
    assert!(!m.contains_key("/avatar/parameters/FT/v2/EyeLeftX"), "eye group is unclaimed");
    assert!(!m.contains_key("/avatar/parameters/VRCEmote"));
    assert!(!m.contains_key("/avatar/parameters/Velocity"));
    assert_eq!(first.len(), 6, "each parameter exactly once: {first:?}");

    // Unchanged frames send nothing; a changed value sends only that parameter.
    assert!(drain(&vrchat, Duration::from_millis(300)).is_empty());
    frame.set_shape(Expression::JawOpen, 0.75);
    out.submit(&frame, claims);
    let delta = drain(&vrchat, Duration::from_millis(300));
    assert_eq!(delta.len(), 1, "{delta:?}");
    assert_eq!(delta[0].0, "/avatar/parameters/FT/v2/JawOpen");
    assert_eq!(delta[0].1, vec![Arg::Float(0.75)]);

    // Periodic full resend within a few seconds.
    let resend = drain(&vrchat, Duration::from_millis(3500));
    assert_eq!(resend.len(), 6, "{resend:?}");

    // Gaining the eye claim sends eye parameters and flips the active flag.
    out.submit(&frame, Claims { eye: true, expression: true });
    let eye = drain(&vrchat, Duration::from_millis(300));
    let m = latest(&eye);
    assert_eq!(m["/avatar/parameters/FT/v2/EyeLeftX"], &Arg::Float(0.25));
    assert_eq!(m["/avatar/parameters/EyeTrackingActive"], &Arg::Bool(true));

    let status = out.status();
    assert!(status.sending, "{status:?}");
    assert!(status.last_error.is_none(), "{status:?}");
    assert_eq!(status.send_addr, vrchat.local_addr().unwrap());

    drop(out);
    let _ = std::fs::remove_dir_all(&root);
}
