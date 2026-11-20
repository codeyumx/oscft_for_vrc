//! Update checks: asks GitHub for the latest OSCFT for VRC release ([`APP_REPO`]) and each installed module's update
//! source (`ModuleView::update_source`: the latest GitHub or GitLab release of its repository, or the version in a
//! JSON document the author publishes) for its latest version, and reports what is newer than the running app /
//! installed module. The two checks are switched on and off separately in `<data_dir>/app-settings.json`, which
//! belongs to the app; `config.json` is owned by the running engine.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fb_core::{ModuleView, UpdateSource};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SETTINGS_FILE: &str = "app-settings.json";
/// A repository's latest release is asked for again after this long.
const RECHECK_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
/// A failed request (offline, rate limited) is retried after this long.
const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest update JSON accepted (`module.json` `UpdateUrl`); it only has to hold a few versions.
const MAX_UPDATE_JSON_BYTES: u64 = 64 * 1024;

/// Settings of the desktop app itself (not the engine).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub check_module_updates: bool,
    /// Check for new OSCFT for VRC releases (independent of the module update check).
    pub check_app_updates: bool,
    /// Closing the window hides it to a tray icon (tracking keeps running); off = no tray icon, closing quits.
    pub close_to_tray: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { check_module_updates: true, check_app_updates: true, close_to_tray: true }
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

/// A module with a newer version at its update source than the installed version.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModuleUpdate {
    pub module_id: String,
    /// The release tag as published (e.g. `v0.7.0`), or the version in the module's update JSON.
    pub latest_version: String,
    /// Where the user gets the update: the release page, or for an update JSON the module's page / download link
    /// (empty when it has neither).
    pub release_url: String,
}

/// The GitHub repository OSCFT for VRC itself is released from.
const APP_REPO: &str = "codeyumx/oscft_for_vrc";

pub fn app_repo() -> UpdateSource {
    UpdateSource::Github { path: APP_REPO.into() }
}

fn is_app_repo(repo: &UpdateSource) -> bool {
    matches!(repo, UpdateSource::Github { path } if path == APP_REPO)
}

/// A newer OSCFT for VRC release than the running app.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    /// The release tag as published (e.g. `v0.4.0`).
    pub latest_version: String,
    /// The release's page, where its downloads are.
    pub release_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Release {
    tag: String,
    url: String,
}

struct Checked {
    due: Instant,
    /// Last known latest release; `None` when the source has none or no request has succeeded yet. A failed
    /// request keeps what was known.
    release: Option<Release>,
}

pub struct UpdateChecker {
    modules_enabled: AtomicBool,
    app_enabled: AtomicBool,
    checked: Mutex<HashMap<UpdateSource, Checked>>,
}

impl UpdateChecker {
    pub fn new(modules_enabled: bool, app_enabled: bool) -> Self {
        Self {
            modules_enabled: AtomicBool::new(modules_enabled),
            app_enabled: AtomicBool::new(app_enabled),
            checked: Mutex::new(HashMap::new()),
        }
    }

    pub fn modules_enabled(&self) -> bool {
        self.modules_enabled.load(Ordering::SeqCst)
    }

    pub fn set_modules_enabled(&self, enabled: bool) {
        self.modules_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn app_enabled(&self) -> bool {
        self.app_enabled.load(Ordering::SeqCst)
    }

    pub fn set_app_enabled(&self, enabled: bool) {
        self.app_enabled.store(enabled, Ordering::SeqCst);
    }

    /// Whether the check that `repo` belongs to (app or modules) is switched on.
    fn wants(&self, repo: &UpdateSource) -> bool {
        if is_app_repo(repo) { self.app_enabled() } else { self.modules_enabled() }
    }

    /// Modules whose update source has a newer version than the installed one (empty while switched off).
    pub fn updates(&self, modules: &[ModuleView]) -> Vec<ModuleUpdate> {
        if !self.modules_enabled() {
            return Vec::new();
        }
        let checked = self.checked.lock().unwrap_or_else(PoisonError::into_inner);
        modules
            .iter()
            .filter_map(|m| {
                let release = checked.get(m.update_source.as_ref()?)?.release.as_ref()?;
                is_newer(&release.tag, m.version.as_deref()?).then(|| ModuleUpdate {
                    module_id: m.id.clone(),
                    latest_version: release.tag.clone(),
                    release_url: release.url.clone(),
                })
            })
            .collect()
    }

    /// The latest OSCFT for VRC release, if it is newer than the running app (`None` while the app check is off).
    pub fn app_update(&self) -> Option<AppUpdate> {
        self.app_update_over(env!("CARGO_PKG_VERSION"))
    }

    fn app_update_over(&self, running: &str) -> Option<AppUpdate> {
        if !self.app_enabled() {
            return None;
        }
        let checked = self.checked.lock().unwrap_or_else(PoisonError::into_inner);
        let release = checked.get(&app_repo())?.release.as_ref()?;
        is_newer(&release.tag, running)
            .then(|| AppUpdate { latest_version: release.tag.clone(), release_url: release.url.clone() })
    }

    /// Asks every source in `repos` that is due for its latest version, and forgets sources no longer listed.
    /// Blocking (network). Returns whether any known release changed.
    pub fn check(&self, repos: &BTreeSet<UpdateSource>) -> bool {
        let due: Vec<UpdateSource> = {
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
            // Switched off while this round was running.
            if !self.wants(&repo) {
                continue;
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

fn latest_release(agent: &ureq::Agent, repo: &UpdateSource) -> Result<Option<Release>> {
    match repo {
        UpdateSource::Github { path } => github_latest_release(agent, path),
        UpdateSource::Gitlab { host, path } => gitlab_latest_release(agent, host, path),
        UpdateSource::Json { url, field, page } => Ok(json_latest_version(agent, url, field)?
            .map(|tag| Release { tag, url: page.clone().unwrap_or_default() })),
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
/// its segments only hold characters that need no other escaping (see `fb_core::ModuleMeta::update_source`).
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

/// `GET` a module's update JSON (`module.json` `UpdateUrl`) and read the version at `field` from it. 404 means
/// nothing is published (yet). The file is anyone's input, so it is only used when it matches the specification
/// exactly (see [`version_in`]); anything else is an error and reports no update.
fn json_latest_version(agent: &ureq::Agent, url: &str, field: &str) -> Result<Option<String>> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .call()
        .context("request failed")?;
    match response.status().as_u16() {
        200 => {
            // Not `read_json`: raw.githubusercontent.com serves JSON files as text/plain.
            let text = response
                .body_mut()
                .with_config()
                .limit(MAX_UPDATE_JSON_BYTES)
                .read_to_string()
                .context("reading the response")?;
            let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).context("the response is not JSON")?;
            let version = version_in(&doc, field)
                .with_context(|| format!("no version string like \"1.2.0\" at \"{field}\""))?;
            Ok(Some(version.to_owned()))
        }
        404 => Ok(None),
        status => anyhow::bail!("answered HTTP {status}"),
    }
}

/// The version in an update JSON: the document must be an object, and the value at `field` (a dot-separated path
/// into nested objects) a string that [`is_spec_version`]. `None` for anything else.
fn version_in<'a>(doc: &'a Value, field: &str) -> Option<&'a str> {
    if !doc.is_object() {
        return None;
    }
    field.split('.').try_fold(doc, |v, key| v.get(key))?.as_str().filter(|s| is_spec_version(s))
}

/// A version as the update JSON specification allows it: 1 to 4 dot-separated decimal numbers of at most 9 digits
/// without leading zeros, optionally prefixed with `v` (`1.0.1`, `v2.10`, `1.0.0.0`). No whitespace, no pre-release
/// or build suffix.
fn is_spec_version(s: &str) -> bool {
    let core = s.strip_prefix('v').unwrap_or(s);
    let mut parts = 0;
    core.split('.').all(|p| {
        parts += 1;
        parts <= 4
            && (1..=9).contains(&p.len())
            && p.bytes().all(|b| b.is_ascii_digit())
            && (p.len() == 1 || !p.starts_with('0'))
    })
}

/// Numeric dot components from the first digit on (`v1.2.0` -> [1, 2, 0]); stops at a pre-release or build
/// suffix. `None` when there is no number.
fn parse_version(s: &str) -> Option<Vec<u64>> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let core = s[start..].split(|c: char| !(c.is_ascii_digit() || c == '.')).next()?;
    core.split('.').filter(|p| !p.is_empty()).map(|p| p.parse().ok()).collect::<Option<Vec<u64>>>().filter(|v| !v.is_empty())
}

/// Whether version `latest` is a higher version than `installed`. Unparsable versions never count as newer.
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
        let custom = AppSettings { check_module_updates: false, check_app_updates: false, close_to_tray: false };
        save_settings(tmp.path(), &custom).unwrap();
        assert_eq!(load_settings(tmp.path()).unwrap(), custom);
        // A file written before a setting existed keeps that setting's default: turning the module check off never
        // turned the app check off.
        std::fs::write(settings_path(tmp.path()), r#"{"checkModuleUpdates":false}"#).unwrap();
        let old = load_settings(tmp.path()).unwrap();
        assert!(!old.check_module_updates && old.check_app_updates && old.close_to_tray);
        std::fs::write(settings_path(tmp.path()), "{").unwrap();
        assert!(load_settings(tmp.path()).is_err());
    }

    #[test]
    fn updates_lists_only_modules_behind_their_source_and_nothing_when_off() {
        let github = |path: &str| UpdateSource::Github { path: path.into() };
        let gitlab = |path: &str| UpdateSource::Gitlab { host: "gitlab.com".into(), path: path.into() };
        let json = |field: &str| UpdateSource::Json {
            url: "https://raw.githubusercontent.com/a/mono/main/versions.json".into(),
            field: field.into(),
            page: None,
        };
        let module = |id: &str, version: Option<&str>, repo: Option<UpdateSource>| ModuleView {
            id: id.into(),
            name: id.into(),
            version: version.map(Into::into),
            author: None,
            description: None,
            usage: None,
            page_url: None,
            update_source: repo,
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
        let checker = UpdateChecker::new(true, true);
        {
            let mut checked = checker.checked.lock().unwrap();
            let release = |tag: &str| Checked { due: Instant::now(), release: Some(Release { tag: tag.into(), url: format!("https://x/{tag}") }) };
            checked.insert(github("a/old"), release("v2.0.0"));
            checked.insert(github("a/current"), release("1.0.0"));
            checked.insert(github("a/none"), Checked { due: Instant::now(), release: None });
            checked.insert(gitlab("g/sub/new"), release("3.1"));
            // One file, two fields: each module of a monorepo has its own entry.
            checked.insert(json("pico.moduleVersion"), release("1.0.1"));
            checked.insert(json("other.moduleVersion"), release("1.0.1"));
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
            module("json-behind", Some("1.0.0"), Some(json("pico.moduleVersion"))),
            module("json-current", Some("1.0.1"), Some(json("other.moduleVersion"))),
        ];
        assert_eq!(
            checker.updates(&modules),
            vec![
                ModuleUpdate { module_id: "behind".into(), latest_version: "v2.0.0".into(), release_url: "https://x/v2.0.0".into() },
                ModuleUpdate { module_id: "gitlab-behind".into(), latest_version: "3.1".into(), release_url: "https://x/3.1".into() },
                ModuleUpdate { module_id: "json-behind".into(), latest_version: "1.0.1".into(), release_url: "https://x/1.0.1".into() },
            ],
        );
        checker.set_app_enabled(false);
        assert_eq!(checker.updates(&modules).len(), 3, "the app check is separate");
        checker.set_modules_enabled(false);
        assert!(checker.updates(&modules).is_empty());
    }

    #[test]
    fn app_update_only_when_the_app_release_is_newer_and_the_app_check_is_on() {
        let checker = UpdateChecker::new(true, true);
        let set_app_release = |tag: Option<&str>| {
            let release = tag.map(|t| Release { tag: t.into(), url: format!("https://x/{t}") });
            checker.checked.lock().unwrap().insert(app_repo(), Checked { due: Instant::now(), release });
        };
        assert_eq!(checker.app_update_over("0.3.1"), None, "not checked yet");
        set_app_release(Some("v0.4.0"));
        assert_eq!(
            checker.app_update_over("0.3.1"),
            Some(AppUpdate { latest_version: "v0.4.0".into(), release_url: "https://x/v0.4.0".into() }),
        );
        assert_eq!(checker.app_update_over("0.4.0"), None, "same version");
        assert_eq!(checker.app_update_over("0.5.0"), None, "running a newer build");
        // A module released from another repository says nothing about the app.
        checker.checked.lock().unwrap().insert(
            UpdateSource::Github { path: "codeyumx/other".into() },
            Checked { due: Instant::now(), release: Some(Release { tag: "v9.0.0".into(), url: String::new() }) },
        );
        assert_eq!(checker.app_update_over("0.4.0"), None);
        checker.set_modules_enabled(false);
        assert!(checker.app_update_over("0.3.1").is_some(), "the module check is separate");
        checker.set_app_enabled(false);
        assert_eq!(checker.app_update_over("0.3.1"), None, "switched off");
        checker.set_app_enabled(true);
        set_app_release(None);
        assert_eq!(checker.app_update_over("0.3.1"), None, "repository has no release");
    }

    #[test]
    fn gitlab_project_path_is_url_encoded_for_the_api() {
        assert_eq!(
            gitlab_latest_release_url("gitlab.example.org", "group/sub/project"),
            "https://gitlab.example.org/api/v4/projects/group%2Fsub%2Fproject/releases/permalink/latest",
        );
    }

    #[test]
    fn only_versions_matching_the_specification_are_read() {
        let doc: Value = serde_json::from_str(
            r#"{"moduleVersion":"1.0.1","appVersion":"v2.10","version":"1.0.0.0","pico":{"moduleVersion":"3"},
                "spaced":" 1.2 ","pre":"1.2.0-beta","build":"1.2+5","number":1.2,"lead":"01.2","five":"1.2.3.4.5",
                "big":"1234567890.0","empty":"","dots":"1..2","trail":"1.","v":"v","vv":"vv1","text":"1.2 click here",
                "list":["1.0"],"obj":{}}"#,
        )
        .unwrap();
        let at = |field| version_in(&doc, field);
        assert_eq!(at("moduleVersion"), Some("1.0.1"));
        assert_eq!(at("appVersion"), Some("v2.10"));
        assert_eq!(at("version"), Some("1.0.0.0"));
        assert_eq!(at("pico.moduleVersion"), Some("3"));
        for rejected in [
            "spaced", "pre", "build", "number", "lead", "five", "big", "empty", "dots", "trail", "v", "vv", "text",
            "list", "list.0", "obj", "pico", "pico.nope", "nope", "moduleVersion.x",
        ] {
            assert_eq!(at(rejected), None, "{rejected}");
        }
        // The document itself must be an object.
        for doc in [r#""1.0.0""#, "1", r#"["1.0.0"]"#, "null"] {
            assert_eq!(version_in(&serde_json::from_str(doc).unwrap(), "version"), None, "{doc}");
        }
    }
}
