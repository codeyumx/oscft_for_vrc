//! Module storage: staging an install from a zip or folder, committing it into `<data_dir>/modules/<id>`,
//! module.json handling.

use crate::config::Paths;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

/// Total uncompressed size accepted from one zip.
const MAX_UNZIPPED_BYTES: u64 = 1 << 30;
const MAX_ZIP_ENTRIES: usize = 20_000;

/// One module found by `FaceBridge.ModuleHost --probe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeHit {
    pub dll: String,
    pub type_name: String,
}

/// Fields of a `module.json` that the engine uses: VRCFT's `TrackingModuleMetadata` plus `Executable`, which marks
/// a native module (an OSCFT-only program speaking the IPC protocol itself, no .NET host). Lenient: missing or
/// mistyped fields are `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleMeta {
    pub module_id: Option<String>,
    pub module_name: Option<String>,
    pub version: Option<String>,
    pub author_name: Option<String>,
    pub module_description: Option<String>,
    pub usage_instructions: Option<String>,
    pub module_page_url: Option<String>,
    pub download_url: Option<String>,
    pub dll_file_name: Option<String>,
    pub executable: Option<String>,
}

impl ModuleMeta {
    pub fn from_value(v: &Value) -> Self {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        Self {
            module_id: s("ModuleId"),
            module_name: s("ModuleName"),
            version: s("Version"),
            author_name: s("AuthorName"),
            module_description: s("ModuleDescription"),
            usage_instructions: s("UsageInstructions"),
            module_page_url: s("ModulePageUrl"),
            download_url: s("DownloadUrl"),
            dll_file_name: s("DllFileName"),
            executable: s("Executable"),
        }
    }

    pub fn read(module_dir: &Path) -> Result<Self> {
        let path = module_dir.join("module.json");
        let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self::from_value(&v))
    }

    /// The GitHub repository the module is published from, as `owner/repo`: taken from `DownloadUrl`, else
    /// `ModulePageUrl`. `None` when neither points at github.com.
    pub fn source_repo(&self) -> Option<String> {
        [&self.download_url, &self.module_page_url].into_iter().flatten().find_map(|u| github_repo(u))
    }
}

/// The program file of a native module whose `module.json` says `"Executable": name`. On Windows `<name>.exe` is
/// preferred when it exists, so one module folder can carry both a Linux `name` and a Windows `name.exe`.
pub fn native_executable(module_dir: &Path, name: &str) -> PathBuf {
    if cfg!(windows) && Path::new(name).extension().is_none() {
        let exe = module_dir.join(format!("{name}.exe"));
        if exe.is_file() {
            return exe;
        }
    }
    module_dir.join(name)
}

fn is_plain_file_name(name: &str) -> bool {
    !name.contains(['/', '\\']) && name != ".." && name != "."
}

/// `owner/repo` from any `https://github.com/<owner>/<repo>[.git][/...]` URL.
fn github_repo(url: &str) -> Option<String> {
    let rest = url.trim();
    let rest = rest.strip_prefix("https://").or_else(|| rest.strip_prefix("http://"))?;
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let (host, path) = rest.split_once('/')?;
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let mut parts = path.split(['/', '?', '#']);
    let owner = parts.next()?;
    let repo = parts.next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (valid(owner) && valid(repo) && repo != "." && repo != "..").then(|| format!("{owner}/{repo}"))
}

/// A fully validated install waiting in a temp directory; [`commit`] moves it into place. Dropping it
/// discards the staging directory.
pub struct Staged {
    pub id: String,
    pub meta: ModuleMeta,
    staging: PathBuf,
    root: PathBuf,
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.staging);
    }
}

/// Copies/extracts `src` (a `.zip` file or a folder) into a staging dir and works out the module: the root
/// folder, what runs it (`module.json` `Executable` for a native module, else the DLL from `DllFileName` or
/// `probe`), the id, and a complete `module.json`. `probe` is never called for native modules.
pub fn stage(paths: &Paths, src: &Path, probe: &dyn Fn(&Path) -> Result<Vec<ProbeHit>>) -> Result<Staged> {
    let staging = paths.staging_dir().join(Uuid::new_v4().simple().to_string());
    let content = staging.join("content");
    fs::create_dir_all(&content).with_context(|| format!("creating {}", content.display()))?;
    let mut staged = Staged { id: String::new(), meta: ModuleMeta::default(), staging, root: content.clone() };
    if src.is_dir() {
        copy_dir(src, &content).with_context(|| format!("copying {}", src.display()))?;
    } else if src.is_file() {
        let file = fs::File::open(src).with_context(|| format!("opening {}", src.display()))?;
        extract_zip(file, &content).with_context(|| format!("extracting {}", src.display()))?;
    } else {
        bail!("{} does not exist", src.display());
    }
    let root = find_module_root(&content)?;
    staged.root = root.clone();

    let json_path = root.join("module.json");
    let existing: Option<Value> = if json_path.is_file() {
        let text = fs::read_to_string(&json_path)?;
        Some(serde_json::from_str(text.trim_start_matches('\u{feff}')).context("module.json is not valid JSON")?)
    } else {
        None
    };
    let meta = existing.as_ref().map(ModuleMeta::from_value).unwrap_or_default();

    let program = match (&meta.executable, &meta.dll_file_name) {
        (Some(_), Some(_)) => {
            bail!("module.json sets both Executable and DllFileName; a module is either native or a .NET DLL")
        }
        (Some(name), None) => {
            if !is_plain_file_name(name) {
                bail!("module.json Executable \"{name}\" is not a plain file name");
            }
            let exe = native_executable(&root, name);
            if !exe.is_file() {
                bail!("module.json names executable \"{name}\" but the module contains no such file");
            }
            // Zips do not keep the executable bit.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&exe, fs::Permissions::from_mode(0o755))
                    .with_context(|| format!("marking {} executable", exe.display()))?;
            }
            name.clone()
        }
        (None, Some(name)) => {
            if !is_plain_file_name(name) {
                bail!("module.json DllFileName \"{name}\" is not a plain file name");
            }
            if !root.join(name).is_file() {
                bail!("module.json names \"{name}\" but the module contains no such file");
            }
            name.clone()
        }
        (None, None) => {
            let hits = probe(&root).context("module has no module.json DllFileName and probing failed")?;
            match hits.as_slice() {
                [] => bail!("no VRCFaceTracking module found in the DLLs"),
                [hit] => {
                    if hit.dll.contains(['/', '\\']) || !root.join(&hit.dll).is_file() {
                        bail!("probe reported \"{}\" which is not in the module folder", hit.dll);
                    }
                    hit.dll.clone()
                }
                many => bail!(
                    "several modules found ({}); install them separately with a module.json DllFileName",
                    many.iter().map(|h| h.type_name.as_str()).collect::<Vec<_>>().join(", ")
                ),
            }
        }
    };

    let id = meta
        .module_id
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::new_v4)
        .hyphenated()
        .to_string();

    let mut obj: Map<String, Value> = match existing {
        Some(Value::Object(o)) => o,
        Some(_) => bail!("module.json is not a JSON object"),
        None => {
            let mut o = Map::new();
            o.insert("IsLocal".into(), Value::Bool(true));
            o
        }
    };
    let before = obj.clone();
    obj.insert("ModuleId".into(), Value::String(id.clone()));
    if meta.executable.is_none() {
        obj.insert("DllFileName".into(), Value::String(program.clone()));
    }
    if meta.module_name.is_none() {
        let stem = Path::new(&program).file_stem().and_then(|s| s.to_str()).unwrap_or(&program).to_owned();
        obj.insert("ModuleName".into(), Value::String(stem));
    }
    if obj != before || !json_path.is_file() {
        fs::write(&json_path, serde_json::to_string_pretty(&Value::Object(obj.clone()))?)
            .context("writing module.json")?;
    }
    staged.meta = ModuleMeta::from_value(&Value::Object(obj));
    staged.id = id;
    Ok(staged)
}

/// Moves a staged module into `<data_dir>/modules/<id>`, replacing any previous install of that id.
pub fn commit(paths: &Paths, staged: Staged) -> Result<()> {
    let dest = paths.module_dir(&staged.id);
    fs::create_dir_all(paths.modules_dir())?;
    remove_dir_retry(&dest).with_context(|| format!("removing previous {}", dest.display()))?;
    fs::rename(&staged.root, &dest).with_context(|| format!("moving module into {}", dest.display()))?;
    Ok(())
}

/// Deletes a module's files and its data dir.
pub fn remove_module_files(paths: &Paths, id: &str) -> Result<()> {
    remove_dir_retry(&paths.module_dir(id))?;
    remove_dir_retry(&paths.module_data_dir(id))
}

/// `remove_dir_all` that tolerates a just-killed process still releasing its files (Windows).
pub(crate) fn remove_dir_retry(path: &Path) -> Result<()> {
    let mut last = None;
    for _ in 0..20 {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(last.map(anyhow::Error::from).unwrap_or_else(|| anyhow::anyhow!("remove failed")))
        .with_context(|| format!("removing {}", path.display()))
}

fn is_module_json(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| n.eq_ignore_ascii_case("module.json"))
}

fn is_dll(name: &std::ffi::OsStr) -> bool {
    Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("dll"))
}

/// The folder holding the module: `dir` itself when it has `module.json` or DLLs, otherwise a single wrapping
/// folder (zips often contain one top-level directory).
fn find_module_root(dir: &Path) -> Result<PathBuf> {
    let mut dir = dir.to_owned();
    for _ in 0..3 {
        let mut subdirs = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let ty = entry.file_type()?;
            if ty.is_file() && (is_module_json(&name) || is_dll(&name)) {
                return Ok(dir);
            }
            if ty.is_dir() && name != "__MACOSX" {
                subdirs.push(entry.path());
            }
        }
        match subdirs.as_slice() {
            [only] => dir = only.clone(),
            _ => break,
        }
    }
    bail!("no module found: expected module.json or a .dll at the top of the archive/folder")
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), &target)?;
        }
        // Symlinks and other special files are not part of a module.
    }
    Ok(())
}

/// Maps a zip entry name to a path under `dest`, rejecting anything that could escape it.
fn safe_entry_path(dest: &Path, name: &str) -> Result<PathBuf> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') {
        bail!("unsafe path in archive: {name}");
    }
    let mut out = dest.to_owned();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains([':', '\0']) {
            bail!("unsafe path in archive: {name}");
        }
        out.push(part);
    }
    // Belt and braces: every component we added must be a plain name.
    let rel = out.strip_prefix(dest).map_err(|_| anyhow::anyhow!("unsafe path in archive: {name}"))?;
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("unsafe path in archive: {name}");
    }
    Ok(out)
}

pub(crate) fn extract_zip<R: Read + io::Seek>(reader: R, dest: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(reader).context("not a valid zip archive")?;
    if archive.len() > MAX_ZIP_ENTRIES {
        bail!("archive has too many entries");
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().context("archive entry has an invalid name")?.into_owned();
        let out = safe_entry_path(dest, &name)?;
        if entry.is_symlink() {
            bail!("archive contains a symbolic link: {name}");
        }
        if entry.is_dir() || name.ends_with('/') || name.ends_with('\\') {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut dst = fs::File::create(&out).with_context(|| format!("creating {}", out.display()))?;
        let budget = MAX_UNZIPPED_BYTES - total;
        let written = io::copy(&mut (&mut entry).take(budget + 1), &mut dst)?;
        total += written;
        if total > MAX_UNZIPPED_BYTES {
            bail!("archive expands to more than {} MiB", MAX_UNZIPPED_BYTES >> 20);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn paths(dir: &Path) -> Paths {
        Paths::new(dir.to_owned())
    }

    fn no_probe(_: &Path) -> Result<Vec<ProbeHit>> {
        panic!("probe must not run when module.json names the DLL");
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    const JSON: &str = r#"{"ModuleId":"103A993C-383F-4A7A-8CB6-5E8B4BF0D133","ModuleName":"Thing","Version":"1.2","AuthorName":"me","DllFileName":"Thing.dll","Extra":5}"#;

    #[test]
    fn zip_with_module_json_installs_under_its_id_and_keeps_extra_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("m.zip");
        make_zip(&zip, &[("module.json", JSON.as_bytes()), ("Thing.dll", b"MZ"), ("lib/dep.dll", b"dep")]);
        let p = paths(&tmp.path().join("data"));
        let staged = stage(&p, &zip, &no_probe).unwrap();
        assert_eq!(staged.id, "103a993c-383f-4a7a-8cb6-5e8b4bf0d133");
        assert_eq!(staged.meta.module_name.as_deref(), Some("Thing"));
        let id = staged.id.clone();
        commit(&p, staged).unwrap();
        let dir = p.module_dir(&id);
        assert!(dir.join("Thing.dll").is_file() && dir.join("lib/dep.dll").is_file());
        let meta: Value = serde_json::from_str(&fs::read_to_string(dir.join("module.json")).unwrap()).unwrap();
        assert_eq!(meta["Extra"], 5);
        assert_eq!(meta["ModuleId"], id.as_str());
        // Staging is cleaned up.
        assert_eq!(fs::read_dir(p.staging_dir()).unwrap().count(), 0);
    }

    #[test]
    fn zip_with_a_single_wrapping_folder_is_unwrapped() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("m.zip");
        make_zip(&zip, &[("Wrapper/module.json", JSON.as_bytes()), ("Wrapper/Thing.dll", b"MZ"), ("__MACOSX/x", b"")]);
        let staged = stage(&paths(&tmp.path().join("data")), &zip, &no_probe).unwrap();
        assert_eq!(staged.meta.dll_file_name.as_deref(), Some("Thing.dll"));
    }

    #[test]
    fn zip_without_module_json_uses_the_probe_and_generates_one() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("Cool Module.zip");
        make_zip(&zip, &[("Cool.dll", b"MZ"), ("Dep.dll", b"MZ")]);
        let p = paths(&tmp.path().join("data"));
        let probe = |dir: &Path| {
            assert!(dir.join("Cool.dll").is_file());
            Ok(vec![ProbeHit { dll: "Cool.dll".into(), type_name: "Ns.Cool".into() }])
        };
        let staged = stage(&p, &zip, &probe).unwrap();
        assert!(Uuid::parse_str(&staged.id).is_ok());
        assert_eq!(staged.meta.dll_file_name.as_deref(), Some("Cool.dll"));
        assert_eq!(staged.meta.module_name.as_deref(), Some("Cool"));
        let id = staged.id.clone();
        commit(&p, staged).unwrap();
        let written = ModuleMeta::read(&p.module_dir(&id)).unwrap();
        assert_eq!(written.module_id.as_deref(), Some(id.as_str()));
        assert_eq!(written.dll_file_name.as_deref(), Some("Cool.dll"));
    }

    #[test]
    fn probe_with_no_or_several_hits_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("m.zip");
        make_zip(&zip, &[("A.dll", b"MZ"), ("B.dll", b"MZ")]);
        let p = paths(&tmp.path().join("data"));
        let none = |_: &Path| Ok(vec![]);
        let err = stage(&p, &zip, &none).err().unwrap();
        assert!(format!("{err:#}").contains("no VRCFaceTracking module"), "{err:#}");
        let two = |_: &Path| {
            Ok(vec![
                ProbeHit { dll: "A.dll".into(), type_name: "A".into() },
                ProbeHit { dll: "B.dll".into(), type_name: "B".into() },
            ])
        };
        let err = stage(&p, &zip, &two).err().unwrap();
        assert!(format!("{err:#}").contains("several modules"), "{err:#}");
        let failing = |_: &Path| -> Result<Vec<ProbeHit>> { bail!("host missing") };
        assert!(stage(&p, &zip, &failing).is_err());
    }

    #[test]
    fn native_module_installs_without_probing_and_becomes_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("m.zip");
        let json = r#"{"ModuleId":"8f0c7a52-1d1e-4a57-9a39-0f4a3b2c1d00","Executable":"demo"}"#;
        make_zip(&zip, &[("module.json", json.as_bytes()), ("demo", b"\x7fELF"), ("demo.exe", b"MZ")]);
        let p = paths(&tmp.path().join("data"));
        let staged = stage(&p, &zip, &no_probe).unwrap();
        assert_eq!(staged.meta.executable.as_deref(), Some("demo"));
        assert_eq!(staged.meta.dll_file_name, None, "a native module must not get a DllFileName");
        assert_eq!(staged.meta.module_name.as_deref(), Some("demo"));
        let id = staged.id.clone();
        commit(&p, staged).unwrap();
        let exe = native_executable(&p.module_dir(&id), "demo");
        assert_eq!(exe.file_name().unwrap(), if cfg!(windows) { "demo.exe" } else { "demo" });
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&exe).unwrap().permissions().mode() & 0o111, 0o111);
        }
    }

    #[test]
    fn native_module_json_is_validated() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(&tmp.path().join("data"));
        let zip = tmp.path().join("m.zip");
        for (json, files, expected) in [
            (r#"{"Executable":"demo","DllFileName":"Thing.dll"}"#, &[("demo", &b"x"[..]), ("Thing.dll", b"MZ")][..], "both"),
            (r#"{"Executable":"missing"}"#, &[("demo", &b"x"[..])][..], "no such file"),
            (r#"{"Executable":"../demo"}"#, &[("demo", &b"x"[..])][..], "plain file name"),
        ] {
            let mut entries = vec![("module.json", json.as_bytes())];
            entries.extend_from_slice(files);
            make_zip(&zip, &entries);
            let err = stage(&p, &zip, &no_probe).err().unwrap_or_else(|| panic!("{json} accepted"));
            assert!(format!("{err:#}").contains(expected), "{json}: {err:#}");
        }
    }

    #[test]
    fn zip_slip_is_rejected_and_nothing_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(&tmp.path().join("data"));
        for evil in ["../evil.txt", "a/../../evil.txt", "..\\evil.txt", "/abs/evil.txt", "C:/evil.txt", "C:\\evil.txt"] {
            let zip = tmp.path().join("evil.zip");
            make_zip(&zip, &[("module.json", JSON.as_bytes()), ("Thing.dll", b"MZ"), (evil, b"pwned")]);
            let err = stage(&p, &zip, &no_probe).err().unwrap_or_else(|| panic!("{evil} accepted"));
            assert!(format!("{err:#}").contains("unsafe path"), "{evil}: {err:#}");
        }
        assert!(!tmp.path().join("evil.txt").exists());
        assert!(!tmp.path().join("data").join("evil.txt").exists());
        assert!(!tmp.path().join("data").join("tmp").join("evil.txt").exists());
        assert_eq!(fs::read_dir(p.staging_dir()).unwrap().count(), 0, "failed installs leave no staging dirs");
    }

    #[test]
    fn dll_file_name_must_exist_and_be_plain() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(&tmp.path().join("data"));
        let zip = tmp.path().join("m.zip");
        make_zip(&zip, &[("module.json", JSON.as_bytes()), ("Other.dll", b"MZ")]);
        assert!(stage(&p, &zip, &no_probe).is_err());
        let traversal = JSON.replace("Thing.dll", "../Thing.dll");
        make_zip(&zip, &[("module.json", traversal.as_bytes()), ("Thing.dll", b"MZ")]);
        assert!(stage(&p, &zip, &no_probe).is_err());
    }

    #[test]
    fn non_zip_and_empty_inputs_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(&tmp.path().join("data"));
        let junk = tmp.path().join("junk.zip");
        fs::write(&junk, b"this is not a zip").unwrap();
        assert!(stage(&p, &junk, &no_probe).is_err());
        let empty = tmp.path().join("empty.zip");
        make_zip(&empty, &[("readme.txt", b"hi")]);
        assert!(format!("{:#}", stage(&p, &empty, &no_probe).err().unwrap()).contains("no module found"));
        assert!(stage(&p, &tmp.path().join("missing.zip"), &no_probe).is_err());
    }

    #[test]
    fn folder_install_copies_and_never_modifies_the_source() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        // No ModuleId and no DllFileName: the copy gets them, the source stays as it was.
        fs::write(src.join("module.json"), r#"{"ModuleName":"X"}"#).unwrap();
        fs::write(src.join("X.dll"), b"MZ").unwrap();
        fs::write(src.join("sub/data.bin"), b"1").unwrap();
        let probe = |_: &Path| Ok(vec![ProbeHit { dll: "X.dll".into(), type_name: "T".into() }]);
        let p = paths(&tmp.path().join("data"));
        let staged = stage(&p, &src, &probe).unwrap();
        let id = staged.id.clone();
        commit(&p, staged).unwrap();
        assert!(p.module_dir(&id).join("sub/data.bin").is_file());
        assert_eq!(fs::read_to_string(src.join("module.json")).unwrap(), r#"{"ModuleName":"X"}"#);
        assert!(src.join("X.dll").is_file());
        // Reinstall replaces; removal deletes files and data.
        fs::create_dir_all(p.module_data_dir(&id)).unwrap();
        remove_module_files(&p, &id).unwrap();
        assert!(!p.module_dir(&id).exists() && !p.module_data_dir(&id).exists());
    }

    #[test]
    fn real_vrcft_zips_install_via_module_json() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-modules");
        let cases = [
            ("FrameEyeOSC_1.0.1-claim-with-packets.zip", "FrameEyeOSC.dll"),
            ("Pico4FaceTracking-VRCFT.zip", "PicoFacialDataModule.dll"),
        ];
        for (zip, dll) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let p = paths(tmp.path());
            let staged = stage(&p, &dir.join(zip), &no_probe).unwrap();
            assert_eq!(staged.meta.dll_file_name.as_deref(), Some(dll));
            assert!(Uuid::parse_str(&staged.id).is_ok());
            let id = staged.id.clone();
            commit(&p, staged).unwrap();
            assert!(p.module_dir(&id).join(dll).is_file());
        }
    }

    #[test]
    fn source_repo_prefers_download_url_and_accepts_only_github_repo_urls() {
        let meta = |download: Option<&str>, page: Option<&str>| ModuleMeta {
            download_url: download.map(Into::into),
            module_page_url: page.map(Into::into),
            ..Default::default()
        };
        let repo = |download, page| meta(download, page).source_repo();
        assert_eq!(
            repo(
                Some("https://github.com/codeyumx/PicoFaceTracking/releases/latest/download/Pico4FaceTracking-VRCFT.zip"),
                Some("https://github.com/someone/else"),
            )
            .as_deref(),
            Some("codeyumx/PicoFaceTracking"),
        );
        assert_eq!(repo(None, Some("https://github.com/sasaken1102r/VRCFT-FrameEyeOSC")).as_deref(), Some("sasaken1102r/VRCFT-FrameEyeOSC"));
        assert_eq!(repo(Some("https://example.com/m.zip"), Some("https://www.github.com/a/b.git#readme")).as_deref(), Some("a/b"));
        assert_eq!(repo(Some("https://github.com/a/b?tab=readme"), None).as_deref(), Some("a/b"));
        for bad in ["https://github.com/onlyowner", "https://github.com//repo", "https://gist.github.com/a/b", "ftp://github.com/a/b", "https://github.com/a/.."] {
            assert_eq!(repo(Some(bad), None), None, "{bad}");
        }
        assert_eq!(repo(None, None), None);
    }
}
