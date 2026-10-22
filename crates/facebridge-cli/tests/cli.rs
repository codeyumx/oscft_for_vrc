//! Runs the real `facebridge-cli` binary against a temporary data dir.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Cli {
    dir: tempfile::TempDir,
}

impl Cli {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Keep `run` off the real network: discard-port target, no listener, no OSCQuery.
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::write(
            dir.path().join("data").join("config.json"),
            r#"{ "osc": { "sendAddr": "127.0.0.1:9", "listenPort": 0, "oscquery": false } }"#,
        )
        .unwrap();
        Self { dir }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_facebridge-cli"))
            .arg("--data-dir")
            .arg(self.data())
            .args(args)
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn fail(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }
}

fn module_folder(root: &Path, id: &str, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("module.json"),
        format!(r#"{{ "ModuleId": "{id}", "ModuleName": "{name}", "Version": "2.0", "DllFileName": "{name}.dll" }}"#),
    )
    .unwrap();
    std::fs::write(dir.join(format!("{name}.dll")), b"x").unwrap();
    dir
}

#[test]
fn install_list_enable_disable_remove() {
    let cli = Cli::new();
    assert!(cli.ok(&["list"]).contains("No modules installed"));

    let id = "5b0e2f9a-6c6b-4f43-9d0a-0c1d2e3f4a5b";
    let folder = module_folder(cli.dir.path(), id, "Sample");
    let out = cli.ok(&["install", folder.to_str().unwrap()]);
    assert!(out.contains("Installed Sample") && out.contains(id), "{out}");

    let list = cli.ok(&["list"]);
    assert!(list.contains("Sample") && list.contains("2.0") && list.contains("enabled"), "{list}");

    // Modules can be addressed by a unique part of the name, case-insensitively.
    assert!(cli.ok(&["disable", "sample"]).contains("Disabled Sample"));
    assert!(cli.ok(&["list"]).contains("disabled"));
    assert!(cli.ok(&["enable", id]).contains("Enabled Sample"));
    assert!(cli.ok(&["list"]).contains("[enabled]"));

    assert!(cli.fail(&["enable", "nothing-like-it"]).contains("no installed module matches"));
    assert!(cli.fail(&["settings", "Sample"]).contains("not declared any settings"));
    assert!(cli.fail(&["install", cli.dir.path().join("missing.zip").to_str().unwrap()]).contains("does not exist"));

    assert!(cli.ok(&["remove", "Sample"]).contains("Removed Sample"));
    assert!(cli.ok(&["list"]).contains("No modules installed"));
    assert!(!cli.data().join("modules").join(id).exists());
}

#[test]
fn settings_roundtrip_from_a_cached_schema() {
    let cli = Cli::new();
    let id = "6c1f3a0b-7d7c-4a54-8e1b-1d2e3f4a5b6c";
    let folder = module_folder(cli.dir.path(), id, "Tunable");
    cli.ok(&["install", folder.to_str().unwrap()]);
    // A running module would have declared its schema; the cache file is how the engine remembers it.
    let data = cli.data().join("module-data").join(id);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("settings-schema.json"),
        r#"{"fields":[
            {"key":"port","label":"UDP port","type":"int","default":11112,"min":1,"max":65535},
            {"key":"mode","label":"Mode","type":"choice","default":"auto","options":[{"value":"auto","label":"A"},{"value":"manual","label":"M"}]}
        ]}"#,
    )
    .unwrap();

    let shown = cli.ok(&["settings", "tunable"]);
    assert!(shown.contains("port = 11112") && shown.contains("mode = \"auto\""), "{shown}");

    let changed = cli.ok(&["settings", "tunable", "port=99999", "mode=manual", "bogus=1"]);
    assert!(changed.contains("port = 65535") && changed.contains("mode = \"manual\""), "{changed}");
    assert!(!changed.contains("bogus"));
    // Persisted across invocations.
    assert!(cli.ok(&["settings", "tunable"]).contains("port = 65535"));
    assert!(cli.fail(&["settings", "tunable", "noequals"]).contains("KEY=VALUE"));
}

#[test]
fn real_vrcft_zip_installs() {
    let cli = Cli::new();
    let zip = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-modules/FrameEyeOSC_1.0.1-claim-with-packets.zip");
    let out = cli.ok(&["install", zip.to_str().unwrap()]);
    assert!(out.contains("Installed FrameEyeOSC") && out.contains("103a993c-383f-4a7a-8cc6-5e8b4bf0d133"), "{out}");
    assert!(cli.ok(&["list"]).contains("sasaken@"));
}

#[test]
fn run_reports_osc_status_and_stops_cleanly() {
    let cli = Cli::new();
    let out = cli.ok(&["run", "--exit-after", "1"]);
    assert!(out.contains("FaceBridge running"), "{out}");
    assert!(out.contains("OSC -> 127.0.0.1:9"), "{out}");
    let shutting = out.find("Shutting down...").expect("shutdown message");
    assert!(out[shutting..].contains("Stopped."), "{out}");
}
