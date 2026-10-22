//! Engine against the real .NET `FaceBridge.ModuleHost` and the real VRCFT module zips in `test-modules/`.
//! Skipped (passes with a note) when the published host is not present or no .NET runtime can be found.

use fb_core::{Engine, EngineSnapshot, ModuleRunState};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn host() -> Option<PathBuf> {
    let p = root().join("dotnet/artifacts/module-host/FaceBridge.ModuleHost.exe");
    p.is_file().then_some(p)
}

/// The machine's .NET 10 runtime location, if `DOTNET_ROOT` is not already set.
fn dotnet_root() -> Option<PathBuf> {
    if std::env::var_os("DOTNET_ROOT").is_some() {
        return None;
    }
    let local = dirs_local()?.join("dotnet10");
    local.is_dir().then_some(local)
}

fn dirs_local() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

fn start(dir: &Path, host: &Path) -> std::sync::Arc<Engine> {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let config = json!({
        "moduleHost": host,
        "dotnetRoot": dotnet_root(),
        "osc": { "sendAddr": "127.0.0.1:9", "listenPort": 0, "oscquery": false },
    });
    std::fs::write(data.join("config.json"), config.to_string()).unwrap();
    Engine::start(data).unwrap()
}

fn wait_for(engine: &Engine, what: &str, pred: impl Fn(&EngineSnapshot) -> bool) -> EngineSnapshot {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let snap = engine.snapshot();
        if pred(&snap) {
            return snap;
        }
        if Instant::now() > deadline {
            let logs: Vec<_> = snap.logs.iter().map(|l| format!("[{}] {}", l.level, l.message)).collect();
            panic!("timed out waiting for {what}\nmodules: {:#?}\nlogs: {logs:#?}", snap.modules);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn real_zips_load_in_the_real_host_and_shut_down_cleanly() {
    let Some(host) = host() else {
        eprintln!("skipped: dotnet/artifacts/module-host/FaceBridge.ModuleHost.exe not built");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let engine = start(tmp.path(), &host);

    let zips = ["FrameEyeOSC_1.0.1-claim-with-packets.zip", "Pico4FaceTracking-VRCFT.zip"];
    let ids: Vec<String> = zips.iter().map(|z| engine.install_module(&root().join("test-modules").join(z)).unwrap()).collect();

    // Each module must get past loading: Initialize may legitimately keep a module in "initializing" (waiting
    // for a headset), but it must not fail.
    for id in &ids {
        let snap = wait_for(&engine, "module to load and initialize", |s| {
            s.modules.iter().find(|m| &m.id == id).is_some_and(|m| {
                matches!(
                    m.state,
                    ModuleRunState::Initializing | ModuleRunState::Running | ModuleRunState::Unclaimed | ModuleRunState::Failed
                )
            })
        });
        let m = snap.modules.iter().find(|m| &m.id == id).unwrap();
        assert_ne!(m.state, ModuleRunState::Failed, "{}: {:?}", m.name, m.last_error);
        assert!(m.supports.eye || m.supports.expression, "{} reported no supported data", m.name);
    }

    let start = Instant::now();
    engine.shutdown();
    assert!(start.elapsed() < Duration::from_secs(15), "shutdown too slow: {:?}", start.elapsed());
    // Every host process is gone. A module that already gave up (claimed nothing, e.g. FrameEyeOSC without
    // packets from the headset) keeps its Unclaimed state; everything else must have been stopped.
    for m in engine.snapshot().modules {
        assert!(matches!(m.state, ModuleRunState::Stopped | ModuleRunState::Unclaimed), "{}: {:?}", m.name, m.state);
    }
}

#[test]
fn zip_without_module_json_is_probed_by_the_real_host() {
    let Some(host) = host() else {
        eprintln!("skipped: module host not built");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    // Re-pack the real module's DLL alone, without module.json.
    let src = tmp.path().join("extract");
    {
        let zip = std::fs::File::open(root().join("test-modules/FrameEyeOSC_1.0.1-claim-with-packets.zip")).unwrap();
        let mut archive = zip::ZipArchive::new(zip).unwrap();
        std::fs::create_dir_all(&src).unwrap();
        let mut dll = archive.by_name("FrameEyeOSC.dll").unwrap();
        std::io::copy(&mut dll, &mut std::fs::File::create(src.join("FrameEyeOSC.dll")).unwrap()).unwrap();
    }
    let engine = start(tmp.path(), &host);
    let id = engine.install_module(&src).unwrap();
    let snap = engine.snapshot();
    let m = snap.modules.iter().find(|m| m.id == id).unwrap();
    assert_eq!(m.name, "FrameEyeOSC");
    let written = std::fs::read_to_string(tmp.path().join("data/modules").join(&id).join("module.json")).unwrap();
    assert!(written.contains("FrameEyeOSC.dll"));
    engine.shutdown();
}
