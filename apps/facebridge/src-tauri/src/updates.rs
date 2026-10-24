//! Module update check: asks GitHub or GitLab for the latest release of each installed module's source repository
//! (`ModuleView::source_repo`) and reports modules whose installed version is older. Switched on and off in
//! `<data_dir>/app-settings.json`, which belongs to the app; `config.json` is owned by the running engine.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fb_core::{ModuleView, SourceRepo};
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
    /// Closing the window hides it to a tray icon (tracking keeps running); off = no tray icon, closing quits.
    pub close_to_tray: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { check_module_updates: true, close_to_tray: true }
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
    checked: Mutex<HashMap<SourceRepo, Checked>>,
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

    /// Asks GitHub / GitLab about every repository in `repos` that is due, and forgets repositories no longer
    /// installed. Blocking (network). Returns whether any known release changed.
    pub fn check(&self, repos: &BTreeSet<SourceRepo>) -> bool {
        let due: Vec<SourceRepo> = {
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

fn latest_release(agent: &ureq::Agent, repo: &SourceRepo) -> Result<Option<Release>> {
    match repo {
        SourceRepo::Github { path } => github_latest_release(agent, path),
        SourceRepo::Gitlab { host, path } => gitlab_latest_release(agent, host, path),
    }
}

const USER_AGENT: &str = concat!("FaceBridge/", env!("CARGO_PKG_VERSION"));

/// `GET /repos/{owner}/{repo}/releases/latest` (newest non-draft, non-prerelease release).
fn github_latest_release(agent: &ureq::Agent, path: &str) -> Result<Option<Release>> {
    #[derive(Deserialize)]
    struct Body {
        tag_name: String,
        html_url: String,
    }
    let mut response = agent
        .get(&format!("https://api.github.com/repos/{path}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", USER_AGENT)
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

/// GitLab's "latest release" endpoint for a project path. The path goes in URL-encoded (`group%2Fsub%2Fproject`);
/// its segments only hold characters that need no other escaping (see `fb_core::ModuleView::source_repo`).
fn gitlab_latest_release_url(host: &str, path: &str) -> String {
    format!("https://{host}/api/v4/projects/{}/releases/permalink/latest", path.replace('/', "%2F"))
}

/// `GET /projects/:id/releases/permalink/latest` (the most recently released release). 404 when the project has no
/// release or is not public.
fn gitlab_latest_release(agent: &ureq::Agent, host: &str, path: &str) -> Result<Option<Release>> {
    #[derive(Deserialize)]
    struct Links {
        #[serde(rename = "self")]
        self_url: Option<String>,
    }
    #[derive(Deserialize)]
    struct Body {
        tag_name: String,
        #[serde(rename = "_links")]
        links: Option<Links>,
    }
    let mut response = agent
        .get(&gitlab_latest_release_url(host, path))
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .call()
        .context("request failed")?;
    match response.status().as_u16() {
        200 => {
            let body: Body = response.body_mut().read_json().context("unexpected response")?;
            let url = body
                .links
                .and_then(|l| l.self_url)
                .unwrap_or_else(|| format!("https://{host}/{path}/-/releases/{}", body.tag_name));
            Ok(Some(Release { tag: body.tag_name, url }))
        }
        404 => Ok(None),
        status => anyhow::bail!("GitLab ({host}) answered HTTP {status}"),
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
        assert_eq!(load_settings(tmp.path()).unwrap(), AppSettings::default());
        let custom = AppSettings { check_module_updates: false, close_to_tray: false };
        save_settings(tmp.path(), &custom).unwrap();
        assert_eq!(load_settings(tmp.path()).unwrap(), custom);
        // A file written before a setting existed keeps that setting's default.
        std::fs::write(settings_path(tmp.path()), r#"{"checkModuleUpdates":false}"#).unwrap();
        assert!(load_settings(tmp.path()).unwrap().close_to_tray);
        std::fs::write(settings_path(tmp.path()), "{").unwrap();
        assert!(load_settings(tmp.path()).is_err());
    }

    #[test]
    fn updates_lists_only_modules_behind_their_release_and_nothing_when_off() {
        let github = |path: &str| SourceRepo::Github { path: path.into() };
        let gitlab = |path: &str| SourceRepo::Gitlab { host: "gitlab.com".into(), path: path.into() };
        let module = |id: &str, version: Option<&str>, repo: Option<SourceRepo>| ModuleView {
            id: id.into(),
            name: id.into(),
            version: version.map(Into::into),
            author: None,
            description: None,
            usage: None,
            page_url: None,
            source_repo: repo,
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
            checked.insert(github("a/old"), release("v2.0.0"));
            checked.insert(github("a/current"), release("1.0.0"));
            checked.insert(github("a/none"), Checked { due: Instant::now(), release: None });
            checked.insert(gitlab("g/sub/new"), release("3.1"));
        }
        let modules = [
            module("behind", Some("1.5"), Some(github("a/old"))),
            module("current", Some("1.0.0"), Some(github("a/current"))),
            module("no-release", Some("1.0"), Some(github("a/none"))),
            module("not-checked-yet", Some("1.0"), Some(github("a/unchecked"))),
            module("no-version", None, Some(github("a/old"))),
            module("no-repo", Some("0.1"), None),
            module("gitlab-behind", Some("3.0"), Some(gitlab("g/sub/new"))),
            // Same path on the other forge is a different repository.
            module("gitlab-unchecked", Some("1.0"), Some(gitlab("a/old"))),
        ];
        assert_eq!(
            checker.updates(&modules),
            vec![
                ModuleUpdate { module_id: "behind".into(), latest_version: "v2.0.0".into(), release_url: "https://x/v2.0.0".into() },
                ModuleUpdate { module_id: "gitlab-behind".into(), latest_version: "3.1".into(), release_url: "https://x/3.1".into() },
            ],
        );
        checker.set_enabled(false);
        assert!(checker.updates(&modules).is_empty());
    }

    #[test]
    fn gitlab_project_path_is_url_encoded_for_the_api() {
        assert_eq!(
            gitlab_latest_release_url("gitlab.example.org", "group/sub/project"),
            "https://gitlab.example.org/api/v4/projects/group%2Fsub%2Fproject/releases/permalink/latest",
        );
    }
}
