//! Management commands. They use the engine in offline mode: nothing is started.

use anyhow::{Context, Result, bail};
use fb_core::{Engine, ModuleView};
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn open(data_dir: PathBuf) -> Result<Arc<Engine>> {
    Engine::open_offline(data_dir)
}

/// Resolves `query` to a module id: exact id, else a unique case-insensitive name match, else a unique id prefix.
pub fn resolve(modules: &[ModuleView], query: &str) -> Result<String> {
    if let Some(m) = modules.iter().find(|m| m.id == query) {
        return Ok(m.id.clone());
    }
    let lower = query.to_lowercase();
    let pick = |matches: Vec<&ModuleView>| -> Result<Option<String>> {
        match matches.as_slice() {
            [] => Ok(None),
            [one] => Ok(Some(one.id.clone())),
            many => bail!(
                "\"{query}\" matches several modules: {}",
                many.iter().map(|m| format!("{} ({})", m.name, m.id)).collect::<Vec<_>>().join(", ")
            ),
        }
    };
    if let Some(id) = pick(modules.iter().filter(|m| m.name.to_lowercase() == lower).collect())? {
        return Ok(id);
    }
    if let Some(id) = pick(modules.iter().filter(|m| m.name.to_lowercase().contains(&lower)).collect())? {
        return Ok(id);
    }
    if let Some(id) = pick(modules.iter().filter(|m| !lower.is_empty() && m.id.starts_with(&lower)).collect())? {
        return Ok(id);
    }
    bail!("no installed module matches \"{query}\" (see `facebridge-cli list`)")
}

pub fn list(data_dir: PathBuf) -> Result<()> {
    let engine = open(data_dir)?;
    let modules = engine.snapshot().modules;
    if modules.is_empty() {
        println!("No modules installed.");
        return Ok(());
    }
    for (i, m) in modules.iter().enumerate() {
        let version = m.version.as_deref().unwrap_or("-");
        let author = m.author.as_deref().map_or(String::new(), |a| format!(" by {a}"));
        println!(
            "{}. {}  {} {version}{author}  [{}]{}",
            i + 1,
            m.id,
            m.name,
            if m.enabled { "enabled" } else { "disabled" },
            if m.has_settings { " settings" } else { "" },
        );
        if let Some(e) = &m.last_error {
            println!("     problem: {e}");
        }
    }
    Ok(())
}

pub fn install(data_dir: PathBuf, path: &Path) -> Result<()> {
    let engine = open(data_dir)?;
    let id = engine.install_module(path)?;
    let name = engine.snapshot().modules.into_iter().find(|m| m.id == id).map_or(id.clone(), |m| m.name);
    println!("Installed {name} ({id})");
    Ok(())
}

pub fn import_vrcft(data_dir: PathBuf) -> Result<()> {
    let engine = open(data_dir)?;
    let ids = engine.import_vrcft_modules()?;
    let modules = engine.snapshot().modules;
    if ids.is_empty() {
        println!("No modules found in the VRCFaceTracking folder.");
    }
    for id in ids {
        let name = modules.iter().find(|m| m.id == id).map_or(id.as_str(), |m| m.name.as_str());
        println!("Imported {name} ({id})");
    }
    Ok(())
}

pub fn remove(data_dir: PathBuf, query: &str) -> Result<()> {
    let engine = open(data_dir)?;
    let modules = engine.snapshot().modules;
    let id = resolve(&modules, query)?;
    let name = modules.iter().find(|m| m.id == id).map_or(id.clone(), |m| m.name.clone());
    engine.uninstall_module(&id)?;
    println!("Removed {name} ({id})");
    Ok(())
}

pub fn set_enabled(data_dir: PathBuf, query: &str, enabled: bool) -> Result<()> {
    let engine = open(data_dir)?;
    let modules = engine.snapshot().modules;
    let id = resolve(&modules, query)?;
    engine.set_module_enabled(&id, enabled)?;
    let name = modules.iter().find(|m| m.id == id).map_or(id.as_str(), |m| m.name.as_str());
    println!("{} {name}", if enabled { "Enabled" } else { "Disabled" });
    Ok(())
}

/// `KEY=VALUE` → (key, JSON value). The value is parsed as JSON when possible, otherwise it is a string.
pub fn parse_assignment(arg: &str) -> Result<(String, Value)> {
    let (key, value) = arg.split_once('=').with_context(|| format!("expected KEY=VALUE, got \"{arg}\""))?;
    if key.is_empty() {
        bail!("expected KEY=VALUE, got \"{arg}\"");
    }
    let value = serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()));
    Ok((key.to_owned(), value))
}

pub fn settings(data_dir: PathBuf, query: &str, assignments: &[String]) -> Result<()> {
    let engine = open(data_dir)?;
    let modules = engine.snapshot().modules;
    let id = resolve(&modules, query)?;
    let name = modules.iter().find(|m| m.id == id).map_or(id.as_str(), |m| m.name.as_str());
    if !assignments.is_empty() {
        let patch: Map<String, Value> = assignments.iter().map(|a| parse_assignment(a)).collect::<Result<_>>()?;
        engine.set_module_settings(&id, Value::Object(patch))?;
    }
    let Some(view) = engine.module_settings(&id) else {
        bail!("{name} has not declared any settings (a module declares them the first time it runs)");
    };
    println!("Settings for {name}:");
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for field in view.schema["fields"].as_array().into_iter().flatten() {
        let key = field["key"].as_str().unwrap_or("?");
        let label = field["label"].as_str().unwrap_or(key);
        let ty = field["type"].as_str().unwrap_or("?");
        let mut range = String::new();
        if let (Some(lo), Some(hi)) = (field.get("min"), field.get("max")) {
            range = format!(" [{lo}..{hi}]");
        }
        if let Some(options) = field["options"].as_array() {
            let values: Vec<_> = options.iter().filter_map(|o| o["value"].as_str()).collect();
            range = format!(" [{}]", values.join(" | "));
        }
        match view.values.get(key) {
            Some(value) => writeln!(out, "  {key} = {value}  ({ty}{range}) {label}")?,
            // Buttons have no value (pressed from the desktop app); unknown future types are listed as-is.
            None => writeln!(out, "  {key}  ({ty}) {label}")?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fb_core::{ModuleRunState, ModuleView};

    fn view(id: &str, name: &str) -> ModuleView {
        ModuleView {
            id: id.into(),
            name: name.into(),
            version: None,
            author: None,
            description: None,
            usage: None,
            page_url: None,
            source_repo: None,
            options: Default::default(),
            enabled: true,
            state: ModuleRunState::Stopped,
            supports: Default::default(),
            claims: Default::default(),
            status: None,
            last_error: None,
            has_settings: false,
            settings_rev: 0,
            image_count: 0,
        }
    }

    #[test]
    fn module_lookup_prefers_exact_id_then_name_then_id_prefix_and_rejects_ambiguity() {
        let mods = [view("aaaa-1", "Pico 4 Face Tracking"), view("bbbb-2", "FrameEyeOSC"), view("bbbc-3", "Pico Eyes")];
        assert_eq!(resolve(&mods, "bbbb-2").unwrap(), "bbbb-2");
        assert_eq!(resolve(&mods, "frameeyeosc").unwrap(), "bbbb-2");
        assert_eq!(resolve(&mods, "frame").unwrap(), "bbbb-2");
        assert_eq!(resolve(&mods, "pico eyes").unwrap(), "bbbc-3");
        assert_eq!(resolve(&mods, "aaaa").unwrap(), "aaaa-1");
        assert!(resolve(&mods, "pico").unwrap_err().to_string().contains("several"));
        assert!(resolve(&mods, "zzz").is_err());
        assert!(resolve(&mods, "").is_err());
    }

    #[test]
    fn assignments_parse_json_values_and_fall_back_to_strings() {
        assert_eq!(parse_assignment("port=11112").unwrap(), ("port".into(), Value::from(11112)));
        assert_eq!(parse_assignment("verbose=true").unwrap().1, Value::Bool(true));
        assert_eq!(parse_assignment("smoothing=0.5").unwrap().1, Value::from(0.5));
        assert_eq!(parse_assignment("host=10.0.0.5").unwrap().1, Value::String("10.0.0.5".into()));
        assert_eq!(parse_assignment("mode=auto").unwrap().1, Value::String("auto".into()));
        assert_eq!(parse_assignment("expr=a=b").unwrap(), ("expr".into(), Value::String("a=b".into())));
        assert!(parse_assignment("novalue").is_err());
        assert!(parse_assignment("=x").is_err());
    }
}
