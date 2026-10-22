//! Module update check: asks GitHub for the latest release of each installed module's source repository
//! (`ModuleView::source_repo`) and reports modules whose installed version is older. Switched on and off in
//! `<data_dir>/app-settings.json`, which belongs to the app; `config.json` is owned by the running engine.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fb_core::ModuleView;
use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "app-settings.json";
/// A repository's latest release is asked for again after this long.
const RECHECK_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
/// A failed request (offline, rate limited) is retried after this long.
const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Settings of the desktop app itself (not the engine).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub check_module_updates: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { check_module_updates: true }
    }
}

fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SETTINGS_FILE)
}

/// Defaults when the file does not exist; an unreadable file is an error.
pub fn load_settings(data_dir: &Path) -> Result<AppSettings> {
    let path = settings_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}')).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn save_settings(data_dir: &Path, settings: &AppSettings) -> Result<()> {
    fb_core::write_json_atomic(&settings_path(data_dir), settings)
}

/// A module with a newer release than the installed version.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModuleUpdate {
    pub module_id: String,
    /// The release tag as published (e.g. `v0.7.0`).
    pub latest_version: String,
    pub release_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Release {
    tag: String,
    url: String,
}

struct Checked {
    due: Instant,
    /// Last known latest release; `None` when the repository has none or no request has succeeded yet. A failed
    /// request keeps what was known.
    release: Option<Release>,
}

pub struct UpdateChecker {
    enabled: AtomicBool,
    checked: Mutex<HashMap<String, Checked>>,
}

impl UpdateChecker {
    pub fn new(enabled: bool) -> Self {
        Self { enabled: AtomicBool::new(enabled), checked: Mutex::new(HashMap::new()) }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Modules whose source has a newer release than the installed version (empty while switched off).
    pub fn updates(&self, modules: &[ModuleView]) -> Vec<ModuleUpdate> {
        if !self.enabled() {
            return Vec::new();
        }
        let checked = self.checked.lock().unwrap_or_else(PoisonError::into_inner);
        modules
            .iter()
            .filter_map(|m| {
                let release = checked.get(m.source_repo.as_ref()?)?.release.as_ref()?;
                is_newer(&release.tag, m.version.as_deref()?).then(|| ModuleUpdate {
                    module_id: m.id.clone(),
                    latest_version: release.tag.clone(),
                    release_url: release.url.clone(),
                })
            })
            .collect()
    }

    /// Asks GitHub about every repository in `repos` that is due, and forgets repositories no longer installed.
    /// Blocking (network). Returns whether any known release changed.
    pub fn check(&self, repos: &BTreeSet<String>) -> bool {
        let due: Vec<String> = {
            let mut checked = self.checked.lock().unwrap_or_else(PoisonError::into_inner);
            checked.retain(|repo, _| repos.contains(repo));
            let now = Instant::now();
            repos.iter().filter(|repo| checked.get(*repo).is_none_or(|c| now >= c.due)).cloned().collect()
        };
        if due.is_empty() {
            return false;
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        let mut changed = false;
        for repo in due {
            if !self.enabled() {
                break;
            }
            let result = latest_release(&agent, &repo);
            let mut checked = self.checked.lock().unwrap_or_else(PoisonError::into_inner);
            let known = checked.remove(&repo).and_then(|c| c.release);
            let entry = match result {
                Ok(release) => Checked { due: Instant::now() + RECHECK_AFTER, release },
                Err(e) => {
                    eprintln!("facebridge: update check for {repo} failed: {e:#}");
                    Checked { due: Instant::now() + RETRY_AFTER, release: known.clone() }
                }
            };
            changed |= entry.release != known;
            checked.insert(repo, entry);
        }
        changed
    }
}

/// `GET /repos/{owner}/{repo}/releases/latest` (newest non-draft, non-prerelease release).
fn latest_release(agent: &ureq::Agent, repo: &str) -> Result<Option<Release>> {
    #[derive(Deserialize)]
    struct Body {
        tag_name: String,
        html_url: String,
    }
    let mut response = agent
        .get(&format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", concat!("FaceBridge/", env!("CARGO_PKG_VERSION")))
        .call()
        .context("request failed")?;
    match response.status().as_u16() {
        200 => {
            let body: Body = response.body_mut().read_json().context("unexpected response")?;
            Ok(Some(Release { tag: body.tag_name, url: body.html_url }))
        }
        404 => Ok(None),
        status => anyhow::bail!("GitHub answered HTTP {status}"),
    }
}

/// Numeric dot components from the first digit on (`v1.2.0` -> [1, 2, 0]); stops at a pre-release or build
/// suffix. `None` when there is no number.
fn parse_version(s: &str) -> Option<Vec<u64>> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let core = s[start..].split(|c: char| !(c.is_ascii_digit() || c == '.')).next()?;
    core.split('.').filter(|p| !p.is_empty()).map(|p| p.parse().ok()).collect::<Option<Vec<u64>>>().filter(|v| !v.is_empty())
}

/// Whether release tag `latest` is a higher version than `installed`. Unparsable versions never count as newer.
fn is_newer(latest: &str, installed: &str) -> bool {
    let (Some(a), Some(b)) = (parse_version(latest), parse_version(installed)) else { return false };
    let len = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..len).map(|i| at(&a, i).cmp(&at(&b, i))).find(|o| o.is_ne()).is_some_and(|o| o.is_gt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_numeric_components_and_ignores_prefixes_and_suffixes() {
        assert!(is_newer("v0.7.0", "0.6.0"));
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(is_newer("release-2.0", "1.99.99"));
        assert!(is_newer("1.0.1", "1.0"));
        assert!(!is_newer("v1.0.1", "1.0.1"));
        assert!(!is_newer("1.0.1.0", "1.0.1"));
        assert!(!is_newer("1.0.0", "1.0.1"));
        assert!(!is_newer("1.2.0-beta", "1.2.0"));
        assert!(!is_newer("latest", "1.0.0"));
        assert!(!is_newer("2.0.0", "unknown"));
    }

    #[test]
    fn settings_default_on_and_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_settings(tmp.path()).unwrap().check_module_updates);
        save_settings(tmp.path(), &AppSettings { check_module_updates: false }).unwrap();
        assert!(!load_settings(tmp.path()).unwrap().check_module_updates);
        std::fs::write(settings_path(tmp.path()), "{").unwrap();
        assert!(load_settings(tmp.path()).is_err());
    }

    #[test]
    fn updates_lists_only_modules_behind_their_release_and_nothing_when_off() {
        let module = |id: &str, version: Option<&str>, repo: Option<&str>| ModuleView {
            id: id.into(),
            name: id.into(),
            version: version.map(Into::into),
            author: None,
            description: None,
            usage: None,
            page_url: None,
            source_repo: repo.map(Into::into),
            enabled: true,
            options: Default::default(),
            state: fb_core::ModuleRunState::Running,
            supports: Default::default(),
            claims: Default::default(),
            status: None,
            last_error: None,
            has_settings: false,
            settings_rev: 0,
            image_count: 0,
        };
        let checker = UpdateChecker::new(true);
        {
            let mut checked = checker.checked.lock().unwrap();
            let release = |tag: &str| Checked { due: Instant::now(), release: Some(Release { tag: tag.into(), url: format!("https://x/{tag}") }) };
            checked.insert("a/old".into(), release("v2.0.0"));
            checked.insert("a/current".into(), release("1.0.0"));
            checked.insert("a/none".into(), Checked { due: Instant::now(), release: None });
        }
        let modules = [
            module("behind", Some("1.5"), Some("a/old")),
            module("current", Some("1.0.0"), Some("a/current")),
            module("no-release", Some("1.0"), Some("a/none")),
            module("not-checked-yet", Some("1.0"), Some("a/unchecked")),
            module("no-version", None, Some("a/old")),
            module("no-repo", Some("0.1"), None),
        ];
        assert_eq!(
            checker.updates(&modules),
            vec![ModuleUpdate { module_id: "behind".into(), latest_version: "v2.0.0".into(), release_url: "https://x/v2.0.0".into() }],
        );
        checker.set_enabled(false);
        assert!(checker.updates(&modules).is_empty());
    }
}
