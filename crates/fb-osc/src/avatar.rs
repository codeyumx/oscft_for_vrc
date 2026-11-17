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

/// OSCQuery `TYPE` without the OSC message's leading comma; Basis writes runtime-published nodes as `",f"`.
/// Only the first tag counts. Strings (`s`), vectors and every other tag are not avatar inputs FaceBridge can drive.
fn osc_type(tag: &str) -> Option<ParamType> {
    match tag.strip_prefix(',').unwrap_or(tag).chars().next()? {
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
    // Leaves without a `TYPE` are skipped and not counted: Basis subscription nodes (`ACCESS 2`, no TYPE/VALUE),
    // and VRChat's tracked bug where a parameter that also has children (`Param` + `Param/Sub`) loses its TYPE.
    let Some(ty) = node.get("TYPE").and_then(Json::as_str) else { return };
    *total += 1;
    // OSCQuery ACCESS: 1 read, 2 write, 3 both. Absent (or not an integer) means writable. Read-only leaves are
    // the game's built-ins (IsLocal, VelocityX, CVR's MovementX, ...) and are counted but never sent.
    let writable = node.get("ACCESS").and_then(Json::as_u64).is_none_or(|a| a & 2 != 0);
    if let (true, Some(ty)) = (writable, osc_type(ty)) {
        out.push((path.to_owned(), ty));
    }
}

/// Extract the avatar from VRChat's OSCQuery tree. `tree` is the response for either `/` or `/avatar`.
/// The id comes from the `/avatar/change` node's `VALUE`. Only `change` and `parameters` are read; other `/avatar`
/// children are ignored: `name` (VRChat 2026.4.1+, Basis), `avatarfullfacetracking` (Basis), `profile` (ChilloutVR).
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

    fn sorted(a: &AvatarParams) -> Vec<(&str, ParamType)> {
        let mut got: Vec<_> = a.params.iter().map(|p| (p.address.as_str(), p.ty)).collect();
        got.sort_unstable_by_key(|p| p.0);
        got
    }

    /// Basis (HVR face tracking, `OsushiNode.cs`): Newtonsoft writes every unset field as `null`, containers have
    /// `ACCESS: 0`, and read-only extras sit next to `parameters` under `/avatar`.
    #[test]
    fn basis_tree_with_null_fields() {
        let leaf = |n: &str| {
            serde_json::json!({ "DESCRIPTION": null, "FULL_PATH": format!("/avatar/parameters/FT/v2/{n}"), "ACCESS": 3,
                "TYPE": "f", "VALUE": [0.0], "CONTENTS": null })
        };
        let container = |path: &str, contents: Json| {
            serde_json::json!({ "DESCRIPTION": null, "FULL_PATH": path, "ACCESS": 0, "TYPE": null, "VALUE": null, "CONTENTS": contents })
        };
        // Runtime-published nodes carry the OSC type tag with its comma.
        let mut published = leaf("MouthClosed");
        published["TYPE"] = ",f".into();
        let v2 = container(
            "/avatar/parameters/FT/v2",
            serde_json::json!({ "JawOpen": leaf("JawOpen"), "EyeY": leaf("EyeY"), "MouthClosed": published }),
        );
        let ft = container("/avatar/parameters/FT", serde_json::json!({ "v2": v2 }));
        let avatar = container(
            "/avatar",
            serde_json::json!({
                "change": { "DESCRIPTION": "", "FULL_PATH": "/avatar/change", "ACCESS": 3, "TYPE": "s",
                    "VALUE": ["avtr_00000000-89b1-4313-aa2d-000000000000"], "CONTENTS": null },
                "name": { "DESCRIPTION": null, "FULL_PATH": "/avatar/name", "ACCESS": 1, "TYPE": "s", "VALUE": ["Basis"], "CONTENTS": null },
                "avatarfullfacetracking": { "DESCRIPTION": null, "FULL_PATH": "/avatar/avatarfullfacetracking", "ACCESS": 1,
                    "TYPE": "T", "VALUE": [true], "CONTENTS": null },
                "parameters": container("/avatar/parameters", serde_json::json!({ "FT": ft })),
            }),
        );
        let root = container("/", serde_json::json!({ "avatar": avatar }));
        let a = parse_oscquery_tree(&root).unwrap();
        assert_eq!(a.id, "avtr_00000000-89b1-4313-aa2d-000000000000");
        assert_eq!(
            sorted(&a),
            [
                ("/avatar/parameters/FT/v2/EyeY", ParamType::Float),
                ("/avatar/parameters/FT/v2/JawOpen", ParamType::Float),
                ("/avatar/parameters/FT/v2/MouthClosed", ParamType::Float),
            ]
        );
        assert_eq!(crate::params::build_plan(&a.params).len(), 3);
    }

    /// VRCFTnyan / VRCFTtoVMCP (`OscJsonServer.cs`): flat `parameters` children keyed by bare name whose `FULL_PATH`
    /// carries the `v2/` segment; `/avatar/change` is read-only.
    #[test]
    fn vrcftnyan_flat_tree_uses_full_path() {
        let tree = serde_json::json!({
            "FULL_PATH": "/avatar", "ACCESS": 0,
            "CONTENTS": {
                "change": { "FULL_PATH": "/avatar/change", "ACCESS": 1, "TYPE": "s", "VALUE": ["avtr_00000000-0000-0000-0000-000000000000"] },
                "parameters": { "FULL_PATH": "/avatar/parameters", "ACCESS": 0, "CONTENTS": {
                    "JawOpen": { "FULL_PATH": "/avatar/parameters/v2/JawOpen", "ACCESS": 3, "TYPE": "f", "VALUE": [0.0] },
                    "MouthSmileLeft": { "FULL_PATH": "/avatar/parameters/v2/MouthSmileLeft", "ACCESS": 3, "TYPE": "f", "VALUE": [0.0] }
                } }
            }
        });
        let a = parse_oscquery_tree(&tree).unwrap();
        assert_eq!(a.id, "avtr_00000000-0000-0000-0000-000000000000");
        assert_eq!(
            sorted(&a),
            [("/avatar/parameters/v2/JawOpen", ParamType::Float), ("/avatar/parameters/v2/MouthSmileLeft", ParamType::Float)]
        );
        assert_eq!(crate::params::build_plan(&a.params).len(), 2);
    }

    /// Resonite VRCFTReceiver mod: no OSCQuery; writes `OSC/vrcft/Avatars/vrc_parameters.json` (id = file name,
    /// not `avtr_*`) and announces `/avatar/change vrc_parameters` on 9001.
    #[test]
    fn resonite_vrcftreceiver_config_file() {
        let root = std::env::temp_dir().join(format!("fb-osc-resonite-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("vrcft").join("Avatars");
        fs::create_dir_all(&dir).unwrap();
        let entry = |n: &str| {
            format!(
                r#"{{"name":"FT/v2/{n}","input":{{"address":"/avatar/parameters/FT/v2/{n}","type":"Float"}},"output":{{"address":"/avatar/parameters/FT/v2/{n}","type":"Float"}}}}"#
            )
        };
        let json = format!(r#"{{"id":"vrc_parameters","name":"expressions (UE)","parameters":[{},{}]}}"#, entry("EyeSquintRight"), entry("JawOpen"));
        fs::write(dir.join("vrc_parameters.json"), json).unwrap();
        let a = load_config(&root, "vrc_parameters").unwrap().unwrap();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(a.name.as_deref(), Some("expressions (UE)"));
        assert_eq!(
            sorted(&a),
            [("/avatar/parameters/FT/v2/EyeSquintRight", ParamType::Float), ("/avatar/parameters/FT/v2/JawOpen", ParamType::Float)]
        );
        assert_eq!(crate::params::build_plan(&a.params).len(), 2);
    }
}
