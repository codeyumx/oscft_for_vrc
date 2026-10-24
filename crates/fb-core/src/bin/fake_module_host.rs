//! Test double for `FaceBridge.ModuleHost`: speaks the IPC protocol, driven by a JSON "script" that stands in for
//! the module DLL. Used only by fb-core's integration tests; not part of the product.
//!
//! Script fields (all optional): `module` (listed by `--probe`), `loadFail`, `supportsEye`, `supportsExpression`,
//! `claimEye`, `claimExpression`, `name`, `images`, `schema` (string), `notify` / `messages` (`[{key,message}]`,
//! sent as `Notify` / `NotifyMessage`), `frames`
//! interval `intervalMs`, `leftGazeX`, `jaw`, `head`, `crashAfterFrames`, `exitAfterFrames`, `hang` (never answer
//! Teardown), `initDelayMs`, `actionWrite` (values JSON string sent as `SettingsWrite` when a button is pressed),
//! `varyEye` / `varyExpression` (default true: that part's data changes every frame; false: constant frames),
//! `freezeAfterFrames` (the data stops changing after that many frames).

use fb_protocol::{
    ChildMessage, FrameFlags, FrameMessage, HostMessage, LogLevel, PROTOCOL_VERSION, ProtocolError,
    read_host_message, write_child_message,
};
use fb_tracking::{Expression, TrackingFrame};
use serde_json::Value;
use std::io::BufReader;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn read_script(path: &str) -> Value {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

fn flag(script: &Value, key: &str, default: bool) -> bool {
    script.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn send(out: &Mutex<TcpStream>, m: &ChildMessage) {
    let mut s = out.lock().unwrap();
    if write_child_message(&mut *s, m).is_err() {
        std::process::exit(6);
    }
}

fn log(out: &Mutex<TcpStream>, message: String) {
    send(
        out,
        &ChildMessage::Log {
            level: LogLevel::Information,
            event_id: 0,
            category: "fake".into(),
            message,
            exception: String::new(),
        },
    );
}

fn probe(dir: &str) {
    let mut modules = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "dll") && flag(&read_script(path.to_str().unwrap()), "module", false) {
            let name = entry.file_name().to_string_lossy().into_owned();
            modules.push(serde_json::json!({ "dll": name, "type": "Fake.Module" }));
        }
    }
    println!("{}", serde_json::json!({ "modules": modules }));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(dir) = arg(&args, "--probe") {
        probe(&dir);
        return;
    }
    let (Some(connect), Some(token), Some(module)) =
        (arg(&args, "--connect"), arg(&args, "--token"), arg(&args, "--module"))
    else {
        std::process::exit(2);
    };
    let script = read_script(&module);
    let stream = TcpStream::connect(&connect).unwrap_or_else(|_| std::process::exit(3));
    let out = Arc::new(Mutex::new(stream.try_clone().unwrap()));
    let mut input = BufReader::new(stream);
    let mut buf = Vec::new();

    send(&out, &ChildMessage::Hello { version: PROTOCOL_VERSION, token, pid: std::process::id() });
    if let Some(reason) = script.get("loadFail").and_then(Value::as_str) {
        send(&out, &ChildMessage::LoadFailed { reason: reason.to_owned() });
        // Close gracefully and give the host time to read: on Windows an abrupt exit can reset the connection
        // and discard data the host has not read yet.
        let _ = out.lock().unwrap().shutdown(std::net::Shutdown::Write);
        std::thread::sleep(Duration::from_millis(300));
        std::process::exit(4);
    }
    if let Some(schema) = script.get("schema").and_then(Value::as_str) {
        send(&out, &ChildMessage::SettingsSchema { json: schema.to_owned() });
    }
    send(
        &out,
        &ChildMessage::Loaded {
            module_type: "Fake.Module".into(),
            supports_eye: flag(&script, "supportsEye", true),
            supports_expression: flag(&script, "supportsExpression", false),
        },
    );

    // Wait for Init, logging whatever Settings arrive first.
    let (eye_available, expression_available) = loop {
        match read_host_message(&mut input, &mut buf) {
            Ok(HostMessage::Settings { json }) => log(&out, format!("settings: {json}")),
            Ok(HostMessage::SettingsAction { key }) => log(&out, format!("action before init: {key}")),
            Ok(HostMessage::Init { eye_available, expression_available }) => break (eye_available, expression_available),
            Ok(HostMessage::Teardown) => {
                send(&out, &ChildMessage::TeardownDone);
                return;
            }
            Err(_) => std::process::exit(6),
        }
    };
    log(&out, format!("init eye={eye_available} expression={expression_available}"));
    if let Some(ms) = script.get("initDelayMs").and_then(Value::as_u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    let claim_eye = flag(&script, "claimEye", true) && eye_available;
    let claim_expression = flag(&script, "claimExpression", false) && expression_available;
    let images: Vec<Vec<u8>> = script
        .get("images")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|i| i.as_array().map(|b| b.iter().map(|x| x.as_u64().unwrap_or(0) as u8).collect()).unwrap_or_default()).collect())
        .unwrap_or_default();
    send(
        &out,
        &ChildMessage::InitResult {
            eye: claim_eye,
            expression: claim_expression,
            name: script.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
            images,
        },
    );
    for (field, persistent) in [("notify", false), ("messages", true)] {
        for n in script.get(field).and_then(Value::as_array).into_iter().flatten() {
            let level = LogLevel::Warning;
            let key = n.get("key").and_then(Value::as_str).unwrap_or("").to_owned();
            let message = n.get("message").and_then(Value::as_str).unwrap_or("").to_owned();
            send(
                &out,
                &if persistent {
                    ChildMessage::NotifyMessage { level, key, message }
                } else {
                    ChildMessage::Notify { level, key, message }
                },
            );
        }
    }

    // Host messages from here on: Settings and button presses are logged, Teardown ends the process.
    let hang = flag(&script, "hang", false);
    let action_write = script.get("actionWrite").and_then(Value::as_str).map(str::to_owned);
    {
        let out = Arc::clone(&out);
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            loop {
                match read_host_message(&mut input, &mut buf) {
                    Ok(HostMessage::Settings { json }) => log(&out, format!("settings: {json}")),
                    Ok(HostMessage::SettingsAction { key }) => {
                        log(&out, format!("action: {key}"));
                        if let Some(json) = &action_write {
                            send(&out, &ChildMessage::SettingsWrite { json: json.clone() });
                        }
                    }
                    Ok(HostMessage::Teardown) if hang => {}
                    Ok(HostMessage::Teardown) => {
                        send(&out, &ChildMessage::TeardownDone);
                        std::process::exit(0);
                    }
                    Ok(HostMessage::Init { .. }) => {}
                    Err(ProtocolError::Closed) | Err(_) => std::process::exit(6),
                }
            }
        });
    }

    let interval = Duration::from_millis(script.get("intervalMs").and_then(Value::as_u64).unwrap_or(5));
    let gaze = script.get("leftGazeX").and_then(Value::as_f64).unwrap_or(0.25) as f32;
    let jaw = script.get("jaw").and_then(Value::as_f64).unwrap_or(0.5) as f32;
    let head = flag(&script, "head", false);
    let crash_after = script.get("crashAfterFrames").and_then(Value::as_u64);
    let exit_after = script.get("exitAfterFrames").and_then(Value::as_u64);
    let vary_eye = flag(&script, "varyEye", true);
    let vary_expression = flag(&script, "varyExpression", true);
    let freeze_after = script.get("freezeAfterFrames").and_then(Value::as_u64).unwrap_or(u64::MAX);
    let mut flags = FrameFlags::default();
    if claim_eye {
        flags = flags | FrameFlags::EYE;
    }
    if claim_expression {
        flags = flags | FrameFlags::EXPRESSION;
    }
    if head {
        flags = flags | FrameFlags::HEAD;
    }
    let mut seq = 0u64;
    loop {
        seq += 1;
        let mut frame = TrackingFrame::default();
        frame.eye.left.gaze_x = gaze;
        frame.eye.left.openness = 1.0;
        frame.set_shape(Expression::JawOpen, jaw);
        // Live data: by default the eye part (right pupil) and the expression part (TongueSquish) change from
        // frame to frame; the knobs freeze one of them, or both after a number of frames.
        let step = (seq.min(freeze_after) % 50) as f32 * 0.01;
        if vary_eye {
            frame.eye.right.pupil_diameter_mm = 3.0 + step;
        }
        if vary_expression {
            frame.set_shape(Expression::TongueSquish, step);
        }
        if head {
            frame.head.yaw = 5.0;
        }
        send(&out, &ChildMessage::Frame(Box::new(FrameMessage { seq, flags, frame })));
        if crash_after == Some(seq) {
            std::process::exit(5);
        }
        if exit_after == Some(seq) {
            std::process::exit(0);
        }
        std::thread::sleep(interval);
    }
}
