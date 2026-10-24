//! Locating `FaceBridge.ModuleHost` (the .NET host for VRCFT modules), launching module processes, and `--probe`.

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

/// Resolves the module host executable, in order: the configured path (`moduleHost` in config.json),
/// `FACEBRIDGE_MODULE_HOST`, `<exe dir>/module-host/`, the host bundled into this binary (extracted under
/// `data_dir`), then the dev build output `dotnet/artifacts/module-host/`.
pub fn discover_module_host(configured: Option<&Path>, data_dir: &Path) -> Result<PathBuf> {
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
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
        tried.push(c.display().to_string());
    }
    if let Some(exe) = extract_bundled_host(data_dir)? {
        return Ok(exe);
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dotnet/artifacts/module-host").join(HOST_EXE);
    if dev.is_file() {
        return Ok(dev);
    }
    tried.push(dev.display().to_string());
    bail!("FaceBridge.ModuleHost not found (set {HOST_ENV_VAR} or module_host in config; looked in: {})", tried.join(", "))
}

/// Extracts the module host bundled into this binary to `<data_dir>/bundled-module-host/<id>/` (`<id>` is a hash of
/// the zip, so each host build gets its own copy) and returns its executable. `None` when no host is bundled.
#[cfg(bundled_host)]
fn extract_bundled_host(data_dir: &Path) -> Result<Option<PathBuf>> {
    const ZIP: &[u8] = include_bytes!(env!("FACEBRIDGE_BUNDLED_HOST_ZIP"));
    const ID: &str = env!("FACEBRIDGE_BUNDLED_HOST_ID");
    let root = data_dir.join("bundled-module-host");
    let installed = root.join(ID);
    let exe = installed.join("module-host").join(HOST_EXE);
    if exe.is_file() {
        return Ok(Some(exe));
    }
    // Extract beside the final folder and rename it into place, so a partial extraction is never used.
    let staging = root.join(format!(".{ID}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    crate::store::extract_zip(std::io::Cursor::new(ZIP), &staging).context("extracting the bundled module host")?;
    let staged_exe = staging.join("module-host").join(HOST_EXE);
    if !staged_exe.is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        bail!("bundled module host zip has no module-host/{HOST_EXE}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged_exe, std::fs::Permissions::from_mode(0o755))?;
    }
    if let Err(e) = std::fs::rename(&staging, &installed) {
        let _ = std::fs::remove_dir_all(&staging);
        if !exe.is_file() {
            return Err(e).with_context(|| format!("installing the bundled module host in {}", installed.display()));
        }
    }
    Ok(Some(exe))
}

#[cfg(not(bundled_host))]
fn extract_bundled_host(_data_dir: &Path) -> Result<Option<PathBuf>> {
    Ok(None)
}

/// Base command for a module process (the .NET module host or a native module) with the environment every launch
/// needs. `dotnet_root` only matters for the .NET host.
pub(crate) fn child_command(program: &Path, dotnet_root: Option<&Path>) -> Command {
    let mut cmd = Command::new(program);
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
    let mut child = child_command(host, dotnet_root)
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
        let dir = tempfile::tempdir().unwrap();
        assert!(discover_module_host(Some(Path::new("Z:/definitely/not/here.exe")), dir.path()).is_err());
        let f = dir.path().join("host.exe");
        std::fs::write(&f, b"").unwrap();
        assert_eq!(discover_module_host(Some(&f), dir.path()).unwrap(), f);
    }

    #[cfg(bundled_host)]
    #[test]
    fn bundled_host_is_extracted_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let first = discover_module_host(None, dir.path()).unwrap();
        assert!(first.starts_with(dir.path()) && first.is_file());
        assert_eq!(discover_module_host(None, dir.path()).unwrap(), first);
    }
}
