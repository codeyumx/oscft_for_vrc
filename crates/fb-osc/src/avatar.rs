//! Avatar parameter discovery inputs: VRChat's per-avatar OSC config files and the OSCQuery `/avatar` tree.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::Value as Json;

use crate::params::{AvatarParam, PARAMETER_PREFIX, ParamType};

/// Everything known about the avatar VRChat has loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvatarParams {
    pub id: String,
    pub name: Option<String>,
    /// Parameters VRChat accepts input for.
    pub params: Vec<AvatarParam>,
    /// Parameters the avatar declares in total (including output-only / read-only ones).
    pub total: usize,
}

fn parse_type(s: &str) -> Option<ParamType> {
    match s.to_ascii_lowercase().as_str() {
        "float" => Some(ParamType::Float),
        "int" => Some(ParamType::Int),
        "bool" => Some(ParamType::Bool),
        _ => None,
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    parameters: Vec<ConfigParam>,
}

#[derive(Deserialize)]
struct ConfigParam {
    input: Option<ConfigEndpoint>,
}

#[derive(Deserialize)]
struct ConfigEndpoint {
    address: Option<String>,
    #[serde(rename = "type")]
    ty: Option<String>,
}

/// Parse a VRChat avatar OSC config (`OSC/<usr_id>/Avatars/<avtr_id>.json`, see
/// <https://docs.vrchat.com/docs/osc-avatar-parameters>). Only `input` endpoints on `/avatar/parameters/...`
/// addresses are usable for sending. `fallback_id` (the file name) is used when the file has no `id`.
pub fn parse_config(json: &str, fallback_id: &str) -> Result<AvatarParams, serde_json::Error> {
    let json = json.strip_prefix('\u{feff}').unwrap_or(json);
    let file: ConfigFile = serde_json::from_str(json)?;
    let total = file.parameters.len();
    let params = file
        .parameters
        .into_iter()
        .filter_map(|p| {
            let input = p.input?;
            let address = input.address?;
            let ty = parse_type(&input.ty?)?;
            address.starts_with(PARAMETER_PREFIX).then_some(AvatarParam { address, ty })
        })
        .collect();
    Ok(AvatarParams { id: file.id.unwrap_or_else(|| fallback_id.to_owned()), name: file.name, params, total })
}

/// `%USERPROFILE%\AppData\LocalLow\VRChat\VRChat\OSC`, or the directory named by `FACEBRIDGE_VRCHAT_OSC_DIR`.
pub fn config_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FACEBRIDGE_VRCHAT_OSC_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(Path::new(&home).join("AppData").join("LocalLow").join("VRChat").join("VRChat").join("OSC"))
}

/// Load the config of avatar `id` from `root`: the most recently written `<root>/<user>/Avatars/<id>.json`.
/// `Ok(None)` when no user has a config for it (e.g. Build & Test avatars, which are never written to disk).
pub fn load_config(root: &Path, id: &str) -> std::io::Result<Option<AvatarParams>> {
    // The id becomes a file name: refuse anything that could escape the directory.
    if id.is_empty() || id.contains(['/', '\\', ':']) || id.contains("..") {
        return Ok(None);
    }
    let file_name = format!("{id}.json");
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    let users = match fs::read_dir(root) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    for user in users.flatten() {
        let path = user.path().join("Avatars").join(&file_name);
        if let Ok(meta) = fs::metadata(&path) {
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, path));
            }
        }
    }
    let Some((_, path)) = newest else { return Ok(None) };
    let text = fs::read_to_string(&path)?;
    parse_config(&text, id).map(Some).map_err(std::io::Error::other)
}

fn osc_type(tag: &str) -> Option<ParamType> {
    match tag.chars().next()? {
        'f' => Some(ParamType::Float),
        'i' => Some(ParamType::Int),
        'T' | 'F' => Some(ParamType::Bool),
        _ => None,
    }
}

fn walk(node: &Json, path: &str, out: &mut Vec<(String, ParamType)>, total: &mut usize) {
    if let Some(children) = node.get("CONTENTS").and_then(Json::as_object) {
        for (name, child) in children {
            let child_path = child.get("FULL_PATH").and_then(Json::as_str).map_or_else(|| format!("{path}/{name}"), str::to_owned);
            walk(child, &child_path, out, total);
        }
        return;
    }
    let Some(ty) = node.get("TYPE").and_then(Json::as_str) else { return };
    *total += 1;
    // OSCQuery ACCESS: 1 read, 2 write, 3 both. Absent means writable.
    let writable = node.get("ACCESS").and_then(Json::as_u64).is_none_or(|a| a & 2 != 0);
    if let (true, Some(ty)) = (writable, osc_type(ty)) {
        out.push((path.to_owned(), ty));
    }
}

/// Extract the avatar from VRChat's OSCQuery tree. `tree` is the response for either `/` or `/avatar`.
/// The id comes from the `/avatar/change` node's `VALUE`.
pub fn parse_oscquery_tree(tree: &Json) -> Option<AvatarParams> {
    let avatar = if tree.get("FULL_PATH").and_then(Json::as_str) == Some("/avatar") {
        tree
    } else {
        tree.get("CONTENTS")?.get("avatar")?
    };
    let contents = avatar.get("CONTENTS")?;
    let id = contents
        .get("change")
        .and_then(|c| c.get("VALUE"))
        .and_then(|v| v.get(0))
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut found = Vec::new();
    let mut total = 0;
    if let Some(parameters) = contents.get("parameters") {
        walk(parameters, "/avatar/parameters", &mut found, &mut total);
    }
    let params = found.into_iter().map(|(address, ty)| AvatarParam { address, ty }).collect();
    Some(AvatarParams { id, name: None, params, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{
        "id" : "avtr_9d58037b-23c7-4c9c-adbd-b1338178cd81",
        "name" : "PurpleMomo",
        "parameters" : [
            { "name" : "Face", "input" : { "address" : "/avatar/parameters/Face", "type" : "Int" },
              "output" : { "address" : "/ableton/trackselect", "type" : "Float" } },
            { "name" : "VelocityZ", "output" : { "address" : "/avatar/parameters/VelocityZ", "type" : "Float" } },
            { "name" : "FT/v2/JawOpen", "input" : { "address" : "/avatar/parameters/FT/v2/JawOpen", "type" : "Float" },
              "output" : { "address" : "/avatar/parameters/FT/v2/JawOpen", "type" : "Float" } },
            { "name" : "FT/v2/JawX2", "input" : { "address" : "/avatar/parameters/FT/v2/JawX2", "type" : "Bool" } },
            { "name" : "Odd", "input" : { "address" : "/other/thing", "type" : "Float" } },
            { "name" : "Weird", "input" : { "address" : "/avatar/parameters/Weird", "type" : "Vector3" } }
        ]
    }"#;

    #[test]
    fn config_file_with_prefixed_names() {
        let a = parse_config(CONFIG, "fallback").unwrap();
        assert_eq!(a.id, "avtr_9d58037b-23c7-4c9c-adbd-b1338178cd81");
        assert_eq!(a.name.as_deref(), Some("PurpleMomo"));
        assert_eq!(a.total, 6);
        let got: Vec<_> = a.params.iter().map(|p| (p.address.as_str(), p.ty)).collect();
        assert_eq!(
            got,
            vec![
                ("/avatar/parameters/Face", ParamType::Int),
                ("/avatar/parameters/FT/v2/JawOpen", ParamType::Float),
                ("/avatar/parameters/FT/v2/JawX2", ParamType::Bool),
            ]
        );
    }

    #[test]
    fn config_with_bom_and_missing_id() {
        let a = parse_config("\u{feff}{\"parameters\":[]}", "avtr_x").unwrap();
        assert_eq!(a.id, "avtr_x");
        assert!(a.params.is_empty());
        assert!(parse_config("not json", "x").is_err());
    }

    #[test]
    fn load_config_picks_newest_user_and_rejects_bad_ids() {
        let root = std::env::temp_dir().join(format!("fb-osc-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (user, name) in [("usr_a", "Old"), ("usr_b", "New")] {
            let dir = root.join(user).join("Avatars");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("avtr_1.json"), format!("{{\"id\":\"avtr_1\",\"name\":\"{name}\",\"parameters\":[]}}")).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        assert_eq!(load_config(&root, "avtr_1").unwrap().unwrap().name.as_deref(), Some("New"));
        assert_eq!(load_config(&root, "avtr_2").unwrap(), None);
        assert_eq!(load_config(&root, "../avtr_1").unwrap(), None);
        assert_eq!(load_config(&root.join("missing"), "avtr_1").unwrap(), None);
        let _ = fs::remove_dir_all(&root);
    }

    fn tree() -> Json {
        serde_json::json!({
            "FULL_PATH": "/",
            "CONTENTS": {
                "avatar": {
                    "FULL_PATH": "/avatar",
                    "CONTENTS": {
                        "change": { "FULL_PATH": "/avatar/change", "TYPE": "s", "ACCESS": 3, "VALUE": ["avtr_77"] },
                        "parameters": {
                            "FULL_PATH": "/avatar/parameters",
                            "CONTENTS": {
                                "VRCEmote": { "FULL_PATH": "/avatar/parameters/VRCEmote", "TYPE": "i", "ACCESS": 3, "VALUE": [0] },
                                "IsLocal": { "FULL_PATH": "/avatar/parameters/IsLocal", "TYPE": "T", "ACCESS": 1, "VALUE": [true] },
                                "FT": {
                                    "FULL_PATH": "/avatar/parameters/FT",
                                    "CONTENTS": { "v2": {
                                        "FULL_PATH": "/avatar/parameters/FT/v2",
                                        "CONTENTS": {
                                            "JawOpen": { "FULL_PATH": "/avatar/parameters/FT/v2/JawOpen", "TYPE": "f", "ACCESS": 3, "VALUE": [0.0] },
                                            "JawX1": { "FULL_PATH": "/avatar/parameters/FT/v2/JawX1", "TYPE": "F", "ACCESS": 3, "VALUE": [false] }
                                        }
                                    } }
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    #[test]
    fn oscquery_tree_with_nested_prefixes() {
        let a = parse_oscquery_tree(&tree()).unwrap();
        assert_eq!(a.id, "avtr_77");
        assert_eq!(a.total, 4);
        let mut got: Vec<_> = a.params.iter().map(|p| (p.address.clone(), p.ty)).collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            vec![
                ("/avatar/parameters/FT/v2/JawOpen".to_owned(), ParamType::Float),
                ("/avatar/parameters/FT/v2/JawX1".to_owned(), ParamType::Bool),
                ("/avatar/parameters/VRCEmote".to_owned(), ParamType::Int),
            ],
            "read-only IsLocal must be excluded"
        );
        // the /avatar subtree on its own parses identically
        let sub = &tree()["CONTENTS"]["avatar"];
        assert_eq!(parse_oscquery_tree(sub), Some(a));
        assert_eq!(parse_oscquery_tree(&serde_json::json!({"FULL_PATH": "/"})), None);
    }
}
