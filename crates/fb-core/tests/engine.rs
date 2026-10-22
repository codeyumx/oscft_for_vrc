//! Engine integration tests against `fb-fake-module-host`, a Rust stand-in for `FaceBridge.ModuleHost` that
//! speaks the real IPC protocol. A module's "DLL" is a JSON script steering the fake (see the fake's docs).

use fb_core::{Engine, EngineEvent, EngineSnapshot, ModuleOptions, ModuleRunState, ModuleView};
use fb_tracking::{Expression, TrackingFrame};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Harness {
    dir: tempfile::TempDir,
    engine: Arc<Engine>,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let config = json!({
            "moduleHost": env!("CARGO_BIN_EXE_fb-fake-module-host"),
            // No real network: send to the discard port, no listener, no OSCQuery.
            "osc": { "sendAddr": "127.0.0.1:9", "listenPort": 0, "rateHz": 30, "oscquery": false },
        });
        std::fs::write(data.join("config.json"), config.to_string()).unwrap();
        let engine = Engine::start(data).unwrap();
        Self { dir, engine }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    /// Creates a module folder whose DLL is `script` and returns the folder.
    fn module_folder(&self, name: &str, script: Value) -> PathBuf {
        let folder = self.dir.path().join("src").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        let id = uuid_for(name);
        std::fs::write(
            folder.join("module.json"),
            json!({ "ModuleId": id, "ModuleName": name, "DllFileName": "Fake.dll", "Version": "1.0" }).to_string(),
        )
        .unwrap();
        std::fs::write(folder.join("Fake.dll"), script.to_string()).unwrap();
        folder
    }

    fn install(&self, name: &str, script: Value) -> String {
        let folder = self.module_folder(name, script);
        self.engine.install_module(&folder).unwrap()
    }

    fn wait(&self, what: &str, pred: impl Fn(&EngineSnapshot) -> bool) -> EngineSnapshot {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let snap = self.engine.snapshot();
            if pred(&snap) {
                return snap;
            }
            if Instant::now() > deadline {
                let logs: Vec<_> = snap.logs.iter().map(|l| format!("[{}] {}", l.level, l.message)).collect();
                let mods: Vec<_> =
                    snap.modules.iter().map(|m| format!("{} {:?} {:?}", m.name, m.state, m.last_error)).collect();
                panic!("timed out waiting for {what}\nmodules: {mods:#?}\nlogs: {logs:#?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_state(&self, id: &str, state: ModuleRunState) -> EngineSnapshot {
        let id = id.to_owned();
        self.wait(&format!("{id} to be {state:?}"), move |s| module(s, &id).state == state)
    }
}

/// Deterministic valid module id per name.
fn uuid_for(name: &str) -> String {
    let mut bytes = [0u8; 16];
    for (i, b) in name.bytes().enumerate() {
        bytes[i % 16] ^= b.wrapping_mul(31).wrapping_add(i as u8);
    }
    uuid::Uuid::from_bytes(bytes).hyphenated().to_string()
}

fn module<'a>(snap: &'a EngineSnapshot, id: &str) -> &'a ModuleView {
    snap.modules.iter().find(|m| m.id == id).unwrap_or_else(|| panic!("module {id} not in snapshot"))
}

fn has_log(snap: &EngineSnapshot, needle: &str) -> bool {
    snap.logs.iter().any(|l| l.message.contains(needle))
}

fn count_logs(snap: &EngineSnapshot, needle: &str) -> usize {
    snap.logs.iter().filter(|l| l.message.contains(needle)).count()
}

#[test]
fn lifecycle_spawn_hello_loaded_init_frames_teardown() {
    let h = Harness::new();
    let events = h.engine.subscribe();
    let id = h.install(
        "EyeMod",
        json!({
            "supportsEye": true, "claimEye": true, "name": "Reported Name",
            "images": [[1, 2, 3], [9]], "leftGazeX": 0.25, "jaw": 0.5,
            "notify": [{ "key": "k", "message": "hello there" }],
            "schema": r#"{"fields":[{"key":"port","label":"Port","type":"int","default":1,"min":1,"max":10}]}"#,
        }),
    );
    let snap = h.wait_state(&id, ModuleRunState::Running);
    let m = module(&snap, &id);
    assert_eq!(m.name, "EyeMod");
    assert_eq!(m.version.as_deref(), Some("1.0"));
    assert!(m.supports.eye && !m.supports.expression);
    assert!(m.claims.eye && !m.claims.expression);
    assert!(m.has_settings);
    assert_eq!(m.image_count, 2);
    assert!(has_log(&snap, "init eye=true expression=true"), "all unclaimed parts are offered");

    assert_eq!(h.engine.module_image(&id, 0), Some(vec![1, 2, 3]));
    assert_eq!(h.engine.module_image(&id, 1), Some(vec![9]));
    assert_eq!(h.engine.module_image(&id, 2), None);

    let snap = h.wait("notification", |s| !s.notifications.is_empty());
    let n = &snap.notifications[0];
    assert_eq!((n.key.as_str(), n.message.as_str(), n.level.as_str()), ("k", "hello there", "warning"));
    assert_eq!((n.module_id.as_str(), n.module_name.as_str()), (id.as_str(), "EyeMod"));

    // Frames reach the merged state: eye from the claimer, shapes stay zero (expression unclaimed).
    let deadline = Instant::now() + Duration::from_secs(20);
    let (frame, claims) = loop {
        let (frame, claims) = h.engine.latest_frame();
        if frame.eye.left.gaze_x == 0.25 || Instant::now() > deadline {
            break (frame, claims);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(frame.eye.left.gaze_x, 0.25);
    assert_eq!(frame.shape(Expression::JawOpen), 0.0);
    assert!(claims.eye && !claims.expression);

    let seen: Vec<_> = events.try_iter().collect();
    assert!(seen.iter().any(|e| matches!(e, EngineEvent::ModulesChanged)));
    assert!(seen.iter().any(|e| matches!(e, EngineEvent::NotificationsChanged)));
    assert!(seen.iter().any(|e| matches!(e, EngineEvent::Log(_))));

    // Dismissal by id.
    h.engine.dismiss_notification(n.id);
    assert!(h.engine.snapshot().notifications.is_empty());

    // Orderly shutdown: Teardown, module Stopped, claim released and data zeroed, idempotent.
    h.engine.shutdown();
    h.engine.shutdown();
    let snap = h.engine.snapshot();
    assert_eq!(module(&snap, &id).state, ModuleRunState::Stopped);
    assert!(snap.notifications.is_empty());
    let (frame, claims) = h.engine.latest_frame();
    assert_eq!(frame, TrackingFrame::default());
    assert!(!claims.eye && !claims.expression);
}

#[test]
fn notifications_are_keyed_and_cleared_when_the_module_stops() {
    let h = Harness::new();
    let id = h.install(
        "Notifier",
        json!({ "notify": [
            { "key": "a", "message": "first" }, { "key": "a", "message": "second" },
            { "key": "", "message": "same text" }, { "key": "", "message": "same text" }
        ] }),
    );
    let snap = h.wait("deduped notifications", |s| s.notifications.len() == 2 && has_log(s, "init"));
    let mut msgs: Vec<_> = snap.notifications.iter().map(|n| n.message.as_str()).collect();
    msgs.sort_unstable();
    assert_eq!(msgs, ["same text", "second"]);
    h.engine.set_module_enabled(&id, false).unwrap();
    let snap = h.wait_state(&id, ModuleRunState::Stopped);
    assert!(snap.notifications.is_empty(), "a stopped module's notifications are removed");
}

#[test]
fn arbitration_follows_priority_and_restart_reassigns() {
    let h = Harness::new();
    let both = json!({ "supportsEye": true, "supportsExpression": true, "claimEye": true, "claimExpression": true });
    let a = h.install("A-eye", json!({ "supportsEye": true, "claimEye": true }));
    let b = h.install("B-both", both);
    let c = h.install("C-eye", json!({ "supportsEye": true, "claimEye": true }));

    // Installs start modules as they arrive; `restart_modules` replays the documented priority order.
    h.engine.restart_modules().unwrap();
    let snap = h.wait("A, B running and C unclaimed", |s| {
        module(s, &a).state == ModuleRunState::Running
            && module(s, &b).state == ModuleRunState::Running
            && module(s, &c).state == ModuleRunState::Unclaimed
    });
    assert!(module(&snap, &a).claims.eye);
    // B is offered only what A left over, and claims just that.
    assert!(!module(&snap, &b).claims.eye && module(&snap, &b).claims.expression);
    assert!(has_log(&snap, "init eye=false expression=true"));
    assert!(module(&snap, &c).supports.eye && !module(&snap, &c).claims.eye);
    let (_, claims) = h.engine.latest_frame();
    assert!(claims.eye && claims.expression);

    // Reordering takes effect on the next restart: C now wins the eye, A is left with nothing.
    h.engine.set_module_order(&[c.clone(), b.clone(), a.clone()]).unwrap();
    assert_eq!(h.engine.snapshot().modules.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), [c.clone(), b.clone(), a.clone()]);
    h.engine.restart_modules().unwrap();
    h.wait("C running, A unclaimed", |s| {
        module(s, &c).state == ModuleRunState::Running && module(s, &a).state == ModuleRunState::Unclaimed
    });
    assert!(h.engine.set_module_order(&["00000000-0000-0000-0000-000000000000".into()]).is_err());
}

#[test]
fn a_crashing_module_releases_its_claim_and_zeroes_its_data() {
    let h = Harness::new();
    let id = h.install(
        "Crasher",
        json!({ "supportsEye": true, "claimEye": true, "crashAfterFrames": 30, "notify": [{ "key": "x", "message": "m" }] }),
    );
    let snap = h.wait_state(&id, ModuleRunState::Failed);
    let m = module(&snap, &id);
    assert!(m.last_error.as_deref().unwrap().contains("exit code 5"), "{:?}", m.last_error);
    assert!(!m.claims.eye);
    assert!(snap.notifications.is_empty());
    let (frame, claims) = h.engine.latest_frame();
    assert_eq!(frame, TrackingFrame::default());
    assert!(!claims.eye && !claims.expression);
    assert!(has_log(&snap, "exit code 5"), "the failure is logged");
    // No automatic reassignment or restart: it stays failed until restart_modules.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(module(&h.engine.snapshot(), &id).state, ModuleRunState::Failed);
}

#[test]
fn clean_exit_without_teardown_is_exited_not_failed() {
    let h = Harness::new();
    let id = h.install("Quitter", json!({ "claimEye": true, "exitAfterFrames": 10 }));
    let snap = h.wait_state(&id, ModuleRunState::Exited);
    assert!(module(&snap, &id).last_error.is_none());
    assert!(!h.engine.latest_frame().1.eye);
}

#[test]
fn load_failure_is_reported() {
    let h = Harness::new();
    let id = h.install("Broken", json!({ "loadFail": "no such type" }));
    let snap = h.wait_state(&id, ModuleRunState::Failed);
    assert!(module(&snap, &id).last_error.as_deref().unwrap().contains("no such type"), "{:?}", module(&snap, &id).last_error);
}

#[test]
fn module_claiming_nothing_is_unclaimed() {
    let h = Harness::new();
    let id = h.install("Idle", json!({ "supportsEye": true, "claimEye": false }));
    let snap = h.wait_state(&id, ModuleRunState::Unclaimed);
    assert!(!module(&snap, &id).claims.eye);
    assert!(module(&snap, &id).supports.eye);
}

#[test]
fn settings_are_validated_persisted_pushed_and_sent_before_init_on_restart() {
    let h = Harness::new();
    let schema = r#"{"fields":[
        {"key":"port","label":"Port","type":"int","default":11112,"min":1,"max":65535},
        {"key":"mode","label":"Mode","type":"choice","default":"auto","options":[{"value":"auto","label":"A"},{"value":"manual","label":"M"}]}
    ]}"#;
    let id = h.install("Configurable", json!({ "claimEye": true, "schema": schema }));
    h.wait_state(&id, ModuleRunState::Running);

    let view = h.engine.module_settings(&id).unwrap();
    assert_eq!(view.values, json!({ "port": 11112, "mode": "auto" }));
    assert_eq!(view.schema["fields"][0]["key"], "port");

    // Out-of-range and unknown values are corrected / dropped; the result is pushed to the running child.
    h.engine.set_module_settings(&id, json!({ "port": 99999, "bogus": 1, "mode": "weird" })).unwrap();
    let expected = json!({ "port": 65535, "mode": "auto" });
    assert_eq!(h.engine.module_settings(&id).unwrap().values, expected);
    let pushed = format!("settings: {}", json!({ "mode": "auto", "port": 65535 }));
    h.wait("settings pushed to the module", |s| has_log(s, &pushed) || has_log(s, &format!("settings: {expected}")));
    let on_disk: Value =
        serde_json::from_str(&std::fs::read_to_string(h.data().join("module-data").join(&id).join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(on_disk, expected);

    // A restart sends the stored values before Init.
    let before = count_logs(&h.engine.snapshot(), "settings: ");
    h.engine.restart_modules().unwrap();
    h.wait("restart with stored settings", |s| {
        module(s, &id).state == ModuleRunState::Running && count_logs(s, "settings: ") > before
    });

    assert!(h.engine.set_module_settings(&id, json!([1])).is_err(), "not an object");
    assert!(h.engine.set_module_settings("00000000-0000-0000-0000-000000000000", json!({})).is_err());
}

#[test]
fn settings_without_schema_are_rejected() {
    let h = Harness::new();
    let id = h.install("Plain", json!({ "claimEye": true }));
    h.wait_state(&id, ModuleRunState::Running);
    assert!(h.engine.module_settings(&id).is_none());
    assert!(h.engine.set_module_settings(&id, json!({ "a": 1 })).is_err());
}

#[test]
fn button_press_reaches_the_module_and_its_display_text_is_shown_but_never_stored() {
    let h = Harness::new();
    let schema = r#"{"fields":[
        {"key":"code","label":"Pairing code","type":"string","default":""},
        {"key":"pair","label":"Pair by code","type":"button"},
        {"key":"status","label":"Status","type":"text","default":"Not paired"}
    ]}"#;
    let id = h.install(
        "Pairer",
        json!({ "claimEye": true, "schema": schema, "actionWrite": r#"{"status":"Paired"}"# }),
    );
    h.wait_state(&id, ModuleRunState::Running);
    assert_eq!(h.engine.module_settings(&id).unwrap().values, json!({ "code": "", "status": "Not paired" }));

    // The user can set the code but not the display text.
    h.engine.set_module_settings(&id, json!({ "code": "4821", "status": "forged" })).unwrap();
    assert_eq!(h.engine.module_settings(&id).unwrap().values, json!({ "code": "4821", "status": "Not paired" }));
    h.wait("only persisted values pushed", |s| has_log(s, &format!("settings: {}", json!({ "code": "4821" }))));

    let rev = module(&h.engine.snapshot(), &id).settings_rev;
    h.engine.press_module_button(&id, "pair").unwrap();
    let snap = h.wait("module updates its status text", |s| {
        has_log(s, "action: pair") && module(s, &id).settings_rev > rev
    });
    assert!(has_log(&snap, "action: pair"));
    assert_eq!(h.engine.module_settings(&id).unwrap().values["status"], json!("Paired"));
    let on_disk: Value =
        serde_json::from_str(&std::fs::read_to_string(h.data().join("module-data").join(&id).join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(on_disk, json!({ "code": "4821" }), "display text is runtime state");

    assert!(h.engine.press_module_button(&id, "code").is_err(), "not a button");
    assert!(h.engine.press_module_button(&id, "missing").is_err());
    h.engine.set_module_enabled(&id, false).unwrap();
    h.wait_state(&id, ModuleRunState::Stopped);
    assert!(h.engine.press_module_button(&id, "pair").unwrap_err().to_string().contains("not running"));
}

#[test]
fn notifications_can_be_turned_off_per_module_and_the_choice_persists() {
    let h = Harness::new();
    let quiet = h.install("Quiet", json!({ "claimEye": true, "notify": [{ "key": "k", "message": "first" }] }));
    let snap = h.wait("first notification", |s| s.notifications.len() == 1);
    assert!(module(&snap, &quiet).options.allow_notifications, "on by default");

    h.engine.set_module_options(&quiet, ModuleOptions { allow_notifications: false }).unwrap();
    let snap = h.engine.snapshot();
    assert!(snap.notifications.is_empty(), "turning notifications off removes the current ones");
    assert!(!module(&snap, &quiet).options.allow_notifications);

    // New notifications from it only reach the log.
    h.engine.restart_modules().unwrap();
    let snap = h.wait("suppressed notification logged", |s| has_log(s, "(notifications off for this module) first"));
    assert!(snap.notifications.is_empty());
    let config: Value = serde_json::from_str(&std::fs::read_to_string(h.data().join("config.json")).unwrap()).unwrap();
    assert_eq!(config["modules"][0]["options"]["allowNotifications"], json!(false));

    h.engine.set_module_options(&quiet, ModuleOptions { allow_notifications: true }).unwrap();
    h.engine.restart_modules().unwrap();
    h.wait("notifications back on", |s| s.notifications.len() == 1);
    assert!(h.engine.set_module_options("00000000-0000-0000-0000-000000000000", ModuleOptions::default()).is_err());
}

#[test]
fn disable_enable_and_uninstall() {
    let h = Harness::new();
    let id = h.install("Toggled", json!({ "claimEye": true }));
    h.wait_state(&id, ModuleRunState::Running);

    h.engine.set_module_enabled(&id, false).unwrap();
    let snap = h.wait_state(&id, ModuleRunState::Stopped);
    assert!(!module(&snap, &id).enabled);
    assert!(!h.engine.latest_frame().1.eye);

    h.engine.set_module_enabled(&id, true).unwrap();
    h.wait_state(&id, ModuleRunState::Running);

    h.engine.uninstall_module(&id).unwrap();
    assert!(h.engine.snapshot().modules.is_empty());
    assert!(!h.data().join("modules").join(&id).exists());
    assert!(h.engine.uninstall_module(&id).is_err());
    assert!(h.engine.set_module_enabled(&id, true).is_err());
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(h.data().join("config.json")).unwrap()).unwrap();
    assert_eq!(cfg["modules"], json!([]));
}

#[test]
fn config_and_modules_survive_an_engine_restart() {
    let h = Harness::new();
    let id = h.install("Persistent", json!({ "claimEye": true }));
    h.wait_state(&id, ModuleRunState::Running);
    h.engine.set_module_enabled(&id, false).unwrap();
    h.wait_state(&id, ModuleRunState::Stopped);
    h.engine.shutdown();
    let data = h.data();
    let engine = Engine::start(data).unwrap();
    let snap = engine.snapshot();
    assert_eq!(snap.modules.len(), 1);
    assert!(!snap.modules[0].enabled);
    assert_eq!(snap.modules[0].state, ModuleRunState::Stopped);
    engine.shutdown();
}

#[test]
fn enabled_modules_start_with_the_engine() {
    let h = Harness::new();
    let id = h.install("Autostart", json!({ "claimEye": true }));
    h.wait_state(&id, ModuleRunState::Running);
    h.engine.shutdown();
    let engine = Engine::start(h.data()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while engine.snapshot().modules[0].state != ModuleRunState::Running {
        assert!(Instant::now() < deadline, "module did not auto-start");
        std::thread::sleep(Duration::from_millis(20));
    }
    engine.shutdown();
}

#[test]
fn shutdown_kills_a_module_that_ignores_teardown_after_five_seconds() {
    let h = Harness::new();
    let id = h.install("Stubborn", json!({ "claimEye": true, "hang": true }));
    h.wait_state(&id, ModuleRunState::Running);
    let start = Instant::now();
    h.engine.shutdown();
    let took = start.elapsed();
    assert!(took >= Duration::from_millis(4500), "gave up on Teardown too early: {took:?}");
    assert!(took < Duration::from_secs(15), "kill took too long: {took:?}");
    assert_eq!(module(&h.engine.snapshot(), &id).state, ModuleRunState::Stopped);
}

#[test]
fn zip_without_module_json_is_installed_via_the_probe() {
    let h = Harness::new();
    let src = h.dir.path().join("loose");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("Module.dll"), json!({ "module": true, "claimEye": true }).to_string()).unwrap();
    std::fs::write(src.join("Dependency.dll"), "{}").unwrap();
    let zip_path = h.dir.path().join("loose.zip");
    {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for name in ["Module.dll", "Dependency.dll"] {
            w.start_file(name, opts).unwrap();
            w.write_all(&std::fs::read(src.join(name)).unwrap()).unwrap();
        }
        w.finish().unwrap();
    }
    let id = h.engine.install_module(&zip_path).unwrap();
    let snap = h.wait_state(&id, ModuleRunState::Running);
    assert_eq!(module(&snap, &id).name, "Module");
    assert!(module_json_dll(&h.data(), &id).ends_with("Module.dll"));

    // Two probe hits are ambiguous.
    std::fs::write(src.join("Dependency.dll"), json!({ "module": true }).to_string()).unwrap();
    let err = h.engine.install_module(&src).unwrap_err();
    assert!(format!("{err:#}").contains("several modules"), "{err:#}");
}

fn module_json_dll(data: &Path, id: &str) -> String {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(data.join("modules").join(id).join("module.json")).unwrap()).unwrap();
    v["DllFileName"].as_str().unwrap().to_owned()
}

#[test]
fn reinstalling_a_running_module_replaces_it() {
    let h = Harness::new();
    let id = h.install("Updatable", json!({ "claimEye": true, "leftGazeX": 0.25 }));
    h.wait_state(&id, ModuleRunState::Running);
    let id2 = h.install("Updatable", json!({ "claimEye": true, "leftGazeX": 0.75 }));
    assert_eq!(id, id2);
    assert_eq!(h.engine.snapshot().modules.len(), 1);
    h.wait_state(&id, ModuleRunState::Running);
    let deadline = Instant::now() + Duration::from_secs(20);
    while h.engine.latest_frame().0.eye.left.gaze_x != 0.75 {
        assert!(Instant::now() < deadline, "new version never produced frames");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn missing_module_host_fails_the_module_not_the_engine() {
    let h = Harness::new();
    let id = h.install("Orphan", json!({ "claimEye": true }));
    h.wait_state(&id, ModuleRunState::Running);
    h.engine.shutdown();
    std::fs::write(
        h.data().join("config.json"),
        json!({
            "moduleHost": "Z:/nowhere/FaceBridge.ModuleHost.exe",
            "osc": { "sendAddr": "127.0.0.1:9", "listenPort": 0, "oscquery": false },
            "modules": [{ "id": id, "enabled": true }],
        })
        .to_string(),
    )
    .unwrap();
    let engine = Engine::start(h.data()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snap = engine.snapshot();
        if snap.modules[0].state == ModuleRunState::Failed {
            assert!(snap.modules[0].last_error.as_deref().unwrap().contains("not found"));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    engine.shutdown();
}

#[test]
fn osc_config_can_be_replaced_and_is_persisted() {
    let h = Harness::new();
    let mut osc = h.engine.snapshot().osc_config;
    osc.send_addr = "127.0.0.1:9100".parse().unwrap();
    h.engine.set_osc_config(osc.clone()).unwrap();
    let snap = h.engine.snapshot();
    assert_eq!(snap.osc_config, osc);
    assert_eq!(snap.osc.send_addr, osc.send_addr);
    let cfg = fb_core::load_config(&h.data()).unwrap();
    assert_eq!(cfg.osc, osc);
}
