//! Tracking mutation through the engine: config persistence, calibration persistence and reset, live learning.

use fb_core::fb_mutate::{AdjustGroup, CalibrationData, MutationConfig, Range};
use fb_core::{Engine, EngineEvent};
use fb_tracking::Expression;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn data_dir(root: &Path) -> PathBuf {
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    data
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn peak(engine: &Engine, shape: Expression) -> f32 {
    engine.calibration().get(shape).unwrap().peak
}

fn wait_for(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !pred() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn mutation_config_is_sanitized_stored_announced_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data = data_dir(dir.path());
    let engine = Engine::open_offline(data.clone()).unwrap();
    assert_eq!(engine.mutation_config(), MutationConfig::default());
    let events = engine.subscribe();

    let mut cfg = MutationConfig::default();
    cfg.filter.enabled = true;
    cfg.calibration.blend = 5.0;
    cfg.filter.head.min_cutoff = 0.0;
    cfg.adjustment.ranges.insert(AdjustGroup::JawOpen, Range { min: 0.9, max: 0.2 });
    let returned = engine.set_mutation_config(cfg).unwrap();
    assert_eq!(returned.calibration.blend, 1.0);
    assert_eq!(returned.filter.head.min_cutoff, 0.01);
    assert!(returned.filter.enabled);
    let jaw = returned.adjustment.range(AdjustGroup::JawOpen);
    assert!(jaw.max - jaw.min >= 0.0099, "{jaw:?}");
    assert_eq!(engine.mutation_config(), returned);
    assert!(events.try_iter().any(|e| matches!(e, EngineEvent::MutationChanged)));

    // config.json carries it, other settings stay.
    let on_disk = read_json(&data.join("config.json"));
    assert_eq!(on_disk["mutation"]["filter"]["enabled"], json!(true));
    assert_eq!(on_disk["mutation"]["filter"]["head"]["minCutoff"], json!(0.01));
    assert!(on_disk["mutation"]["adjustment"]["ranges"]["jawOpen"].is_object());
    drop(engine);

    let engine = Engine::open_offline(data).unwrap();
    assert_eq!(engine.mutation_config(), returned);
}

#[test]
fn calibration_loads_resets_by_name_and_reports_unknown_names() {
    let dir = tempfile::tempdir().unwrap();
    let data = data_dir(dir.path());
    let path = data.join("calibration.json");
    std::fs::write(
        &path,
        json!({ "shapes": [
            { "name": "JawOpen", "peak": 0.8, "progress": 0.5 },
            { "name": "EyeWideRight", "peak": 0.99, "progress": 1.0 },
            { "name": "NoSuchShape", "peak": 1.0, "progress": 1.0 },
        ] })
        .to_string(),
    )
    .unwrap();

    let engine = Engine::open_offline(data.clone()).unwrap();
    let events = engine.subscribe();
    assert_eq!(peak(&engine, Expression::JawOpen), 0.8);
    assert_eq!(peak(&engine, Expression::EyeWideRight), 0.99);
    assert_eq!(engine.calibration().shapes.len(), fb_tracking::SHAPE_COUNT, "one entry per shape");

    let err = engine.reset_calibration(Some("NoSuchShape")).unwrap_err();
    assert!(format!("{err:#}").contains("NoSuchShape"), "{err:#}");
    assert_eq!(peak(&engine, Expression::JawOpen), 0.8, "a failed reset changes nothing");

    engine.reset_calibration(Some("JawOpen")).unwrap();
    assert_eq!(peak(&engine, Expression::JawOpen), 0.0);
    assert_eq!(peak(&engine, Expression::EyeWideRight), 0.99);
    assert!(events.try_iter().any(|e| matches!(e, EngineEvent::MutationChanged)));
    // Persisted immediately.
    let saved: CalibrationData = serde_json::from_value(read_json(&path)).unwrap();
    assert_eq!(saved.get(Expression::JawOpen).unwrap().peak, 0.0);
    assert_eq!(saved.get(Expression::EyeWideRight).unwrap().peak, 0.99);
    drop(engine);

    let engine = Engine::open_offline(data).unwrap();
    assert_eq!(peak(&engine, Expression::JawOpen), 0.0);
    engine.reset_calibration(None).unwrap();
    assert_eq!(peak(&engine, Expression::EyeWideRight), 0.0);
    let saved: CalibrationData = serde_json::from_value(read_json(&path)).unwrap();
    assert_eq!(saved.mean_progress(), 0.0);
}

#[test]
fn corrupt_calibration_file_means_empty_data_and_is_logged() {
    let dir = tempfile::tempdir().unwrap();
    let data = data_dir(dir.path());
    std::fs::write(data.join("calibration.json"), "{ not json").unwrap();
    let engine = Engine::open_offline(data).unwrap();
    assert_eq!(peak(&engine, Expression::JawOpen), 0.0);
    let snap = engine.snapshot();
    assert!(snap.logs.iter().any(|l| l.message.contains("corrupt") && l.message.contains("calibration.json")));
}

#[test]
fn calibration_learns_from_live_frames_and_is_saved_on_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let data = data_dir(dir.path());
    let host = env!("CARGO_BIN_EXE_fb-fake-module-host");
    std::fs::write(
        data.join("config.json"),
        json!({
            "moduleHost": host,
            // No real network: discard port, no listener, no OSCQuery.
            "osc": { "sendAddr": "127.0.0.1:9", "listenPort": 0, "rateHz": 30, "oscquery": false },
            "mutation": { "calibration": { "enabled": true } },
        })
        .to_string(),
    )
    .unwrap();

    let module = dir.path().join("module");
    std::fs::create_dir_all(&module).unwrap();
    std::fs::write(
        module.join("module.json"),
        json!({
            "ModuleId": "5f2b1c0e-7a43-4c88-9d11-3e6f0a9b2c77",
            "ModuleName": "Jaw",
            "DllFileName": "Fake.dll",
            "Version": "1.0",
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        module.join("Fake.dll"),
        json!({ "supportsExpression": true, "claimEye": false, "claimExpression": true, "jaw": 0.6 }).to_string(),
    )
    .unwrap();

    let engine = Engine::start(data.clone()).unwrap();
    engine.install_module(&module).unwrap();
    wait_for("jaw progress", || {
        let jaw = engine.calibration().get(Expression::JawOpen).cloned().unwrap();
        jaw.peak > 0.4 && jaw.progress > 0.05
    });

    // The output is calibrated, the raw frame is what the module sent.
    let (raw, _) = engine.latest_raw_frame();
    let (out, claims) = engine.latest_frame();
    assert!(claims.expression);
    assert_eq!(raw.shape(Expression::JawOpen), 0.6);
    assert!(out.shape(Expression::JawOpen) > 0.6 && out.shape(Expression::JawOpen) <= 1.0);

    engine.shutdown();
    let learned = engine.calibration();
    assert!(learned.get(Expression::JawOpen).unwrap().peak > 0.4);
    let saved: CalibrationData = serde_json::from_value(read_json(&data.join("calibration.json"))).unwrap();
    assert_eq!(saved, learned);
    drop(engine);

    // A restart starts from what was learned.
    let engine = Engine::open_offline(data).unwrap();
    assert_eq!(engine.calibration(), learned);
}
