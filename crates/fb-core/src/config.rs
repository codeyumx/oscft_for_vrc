//! `<data_dir>/config.json` and the on-disk module layout.

use anyhow::{Context, Result};
use fb_osc::OscConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleEntry {
    pub id: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub options: ModuleOptions,
}

impl ModuleEntry {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), enabled: true, options: ModuleOptions::default() }
    }
}

fn enabled_by_default() -> bool {
    true
}

/// Options FaceBridge offers for every module, independent of what the module itself declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModuleOptions {
    /// Show the module's notifications (banners and pop-ups). When off they only go to the log.
    pub allow_notifications: bool,
}

impl Default for ModuleOptions {
    fn default() -> Self {
        Self { allow_notifications: true }
    }
}

/// Persisted engine configuration. `modules` order is the claim priority.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineConfig {
    pub osc: OscConfig,
    /// `FaceBridge.ModuleHost(.exe)`; `None` = discover.
    pub module_host: Option<PathBuf>,
    /// Passed to module hosts as `DOTNET_ROOT` when set.
    pub dotnet_root: Option<PathBuf>,
    pub modules: Vec<ModuleEntry>,
}

/// `%APPDATA%\FaceBridge` on Windows, `<config dir>/FaceBridge` elsewhere.
pub fn default_data_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("FaceBridge")
}

pub(crate) fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("config.json")
}

/// Loads `<data_dir>/config.json`; defaults when the file does not exist. A corrupt file is an error (it is
/// never silently overwritten).
pub fn load_config(data_dir: &Path) -> Result<EngineConfig> {
    let path = config_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(EngineConfig::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Writes `<data_dir>/config.json` atomically (temp file + rename).
pub fn save_config(data_dir: &Path, cfg: &EngineConfig) -> Result<()> {
    write_json_atomic(&config_path(data_dir), cfg)
}

/// Writes `value` as pretty JSON to `path` atomically (temp file + rename), creating the parent directory.
pub fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let dir = path.parent().context("path has no parent directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let text = serde_json::to_string_pretty(value)?;
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Locations under the data dir.
#[derive(Clone, Debug)]
pub(crate) struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    pub fn modules_dir(&self) -> PathBuf {
        self.data_dir.join("modules")
    }

    pub fn module_dir(&self, id: &str) -> PathBuf {
        self.modules_dir().join(id)
    }

    pub fn module_data_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("module-data").join(id)
    }

    pub fn settings_path(&self, id: &str) -> PathBuf {
        self.module_data_dir(id).join("settings.json")
    }

    pub fn schema_cache_path(&self, id: &str) -> PathBuf {
        self.module_data_dir(id).join("settings-schema.json")
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_round_trip_preserves_everything() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_config(dir.path()).unwrap(), EngineConfig::default());
        let cfg = EngineConfig {
            module_host: Some(PathBuf::from("C:/host/FaceBridge.ModuleHost.exe")),
            dotnet_root: Some(PathBuf::from("C:/dotnet")),
            modules: vec![
                ModuleEntry::new("a"),
                ModuleEntry { enabled: false, options: ModuleOptions { allow_notifications: false }, ..ModuleEntry::new("b") },
            ],
            ..EngineConfig::default()
        };
        save_config(dir.path(), &cfg).unwrap();
        assert_eq!(load_config(dir.path()).unwrap(), cfg);
        // Saving again replaces the existing file and leaves no temp file behind.
        save_config(dir.path(), &EngineConfig::default()).unwrap();
        assert_eq!(load_config(dir.path()).unwrap(), EngineConfig::default());
        assert!(!dir.path().join("config.json.tmp").exists());
    }

    #[test]
    fn partial_and_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(config_path(dir.path()), r#"{"modules":[{"id":"x"}],"unknownKey":1}"#).unwrap();
        let cfg = load_config(dir.path()).unwrap();
        assert_eq!(cfg.modules, vec![ModuleEntry::new("x")]);
        assert_eq!(cfg.osc, OscConfig::default());
        std::fs::write(config_path(dir.path()), "{ not json").unwrap();
        assert!(load_config(dir.path()).is_err());
    }
}
