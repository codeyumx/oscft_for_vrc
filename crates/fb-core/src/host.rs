//! Locating `FaceBridge.ModuleHost`, launching it, and `--probe` mode.

use crate::store::ProbeHit;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const HOST_ENV_VAR: &str = "FACEBRIDGE_MODULE_HOST";

const HOST_EXE: &str = if cfg!(windows) { "FaceBridge.ModuleHost.exe" } else { "FaceBridge.ModuleHost" };

/// Windows `CREATE_NO_WINDOW`: no console window for the child.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Resolves the module host executable: explicit config path, `FACEBRIDGE_MODULE_HOST`,
/// `<exe dir>/module-host/`, then the dev build output `dotnet/artifacts/module-host/`.
pub fn discover_module_host(configured: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = configured {
        if p.is_file() {
            return Ok(p.to_owned());
        }
        bail!("configured module host not found: {}", p.display());
    }
    let mut tried = Vec::new();
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os(HOST_ENV_VAR).filter(|v| !v.is_empty()) {
        candidates.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("module-host").join(HOST_EXE));
        }
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dotnet/artifacts/module-host").join(HOST_EXE));
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
        tried.push(c.display().to_string());
    }
    bail!("FaceBridge.ModuleHost not found (set {HOST_ENV_VAR} or module_host in config; looked in: {})", tried.join(", "))
}

/// Base command for the module host with the environment every launch needs.
pub(crate) fn host_command(host: &Path, dotnet_root: Option<&Path>) -> Command {
    let mut cmd = Command::new(host);
    if let Some(root) = dotnet_root {
        cmd.env("DOTNET_ROOT", root);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Deserialize)]
struct ProbeOutput {
    modules: Vec<ProbeEntry>,
}

#[derive(Deserialize)]
struct ProbeEntry {
    dll: String,
    #[serde(rename = "type")]
    type_name: String,
}

pub(crate) fn parse_probe_output(stdout: &str) -> Result<Vec<ProbeHit>> {
    let line = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('{'))
        .context("module host --probe printed no JSON")?;
    let out: ProbeOutput = serde_json::from_str(line).context("parsing --probe output")?;
    Ok(out.modules.into_iter().map(|m| ProbeHit { dll: m.dll, type_name: m.type_name }).collect())
}

/// Runs `FaceBridge.ModuleHost --probe <dir>` (30 s limit) and returns the modules it found.
pub fn probe_modules(host: &Path, dotnet_root: Option<&Path>, dir: &Path) -> Result<Vec<ProbeHit>> {
    let mut child = host_command(host, dotnet_root)
        .arg("--probe")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", host.display()))?;
    let mut stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let out_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("module host --probe timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    if !status.success() {
        bail!("module host --probe failed ({status}): {}", stderr.trim());
    }
    parse_probe_output(&stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_output_takes_the_json_line_and_ignores_noise() {
        let hits = parse_probe_output(
            "warming up\n{\"modules\":[{\"dll\":\"A.dll\",\"type\":\"Ns.A\"},{\"dll\":\"B.dll\",\"type\":\"Ns.B\"}]}\n",
        )
        .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[1].dll.as_str(), hits[1].type_name.as_str()), ("B.dll", "Ns.B"));
        assert!(parse_probe_output("nothing here").is_err());
        assert!(parse_probe_output("{\"modules\": 3}").is_err());
    }

    #[test]
    fn configured_host_must_exist() {
        assert!(discover_module_host(Some(Path::new("Z:/definitely/not/here.exe"))).is_err());
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("host.exe");
        std::fs::write(&f, b"").unwrap();
        assert_eq!(discover_module_host(Some(&f)).unwrap(), f);
    }
}
