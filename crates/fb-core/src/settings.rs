//! Module settings: schema parsing (`spec/ipc-protocol.md`, "Settings schema JSON") and value validation.
//!
//! Validation rules: unknown keys are dropped, values of the wrong type are dropped, numbers are clamped to
//! `min`/`max`, and missing keys are filled with the field's `default`. Field types this version does not know
//! are skipped (a newer SDK must not make the whole form unusable).
//!
//! Two field types carry no user value: `text` is display text owned by the module (the user cannot change it,
//! it is not persisted, and each declaration resets it to the declared text), and `button` has no value at all.

use serde_json::{Map, Number, Value};
use std::collections::HashSet;

#[derive(Clone, Debug)]
enum Kind {
    Int,
    Float,
    Bool,
    Text,
    Choice(Vec<String>),
    /// Read-only display text, written only by the module.
    Display,
    Button,
}

#[derive(Clone, Debug)]
struct Field {
    key: String,
    kind: Kind,
    default: Value,
    min: Option<f64>,
    max: Option<f64>,
}

impl Field {
    fn has_value(&self) -> bool {
        !matches!(self.kind, Kind::Button)
    }

    fn user_editable(&self) -> bool {
        !matches!(self.kind, Kind::Button | Kind::Display)
    }
}

#[derive(Clone, Debug)]
pub struct Schema {
    /// The schema exactly as the module declared it (what the UI renders).
    pub raw: Value,
    fields: Vec<Field>,
}

impl Schema {
    pub fn parse(json: &str) -> Result<Schema, String> {
        let raw: Value = serde_json::from_str(json).map_err(|e| format!("schema is not valid JSON: {e}"))?;
        let list = raw
            .get("fields")
            .and_then(Value::as_array)
            .ok_or("schema has no \"fields\" array")?;
        let mut fields = Vec::with_capacity(list.len());
        let mut seen = HashSet::new();
        for f in list {
            let key = f.get("key").and_then(Value::as_str).filter(|k| !k.is_empty());
            let key = key.ok_or("field without a \"key\"")?.to_owned();
            if !seen.insert(key.clone()) {
                return Err(format!("duplicate field key \"{key}\""));
            }
            let kind = match f.get("type").and_then(Value::as_str) {
                Some("int") => Kind::Int,
                Some("float") => Kind::Float,
                Some("bool") => Kind::Bool,
                Some("string") => Kind::Text,
                Some("text") => Kind::Display,
                Some("button") => Kind::Button,
                Some("choice") => {
                    let options: Vec<String> = f
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|o| {
                            o.iter()
                                .filter_map(|o| o.get("value").and_then(Value::as_str).map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    if options.is_empty() {
                        return Err(format!("choice field \"{key}\" has no options"));
                    }
                    Kind::Choice(options)
                }
                Some(_) => continue,
                None => return Err(format!("field \"{key}\" has no \"type\"")),
            };
            let number = |name: &str| f.get(name).and_then(Value::as_f64);
            let (min, max) = (number("min"), number("max"));
            if matches!((min, max), (Some(lo), Some(hi)) if lo > hi) {
                return Err(format!("field \"{key}\": min is greater than max"));
            }
            let mut field = Field { key: key.clone(), kind, default: Value::Null, min, max };
            if field.has_value() {
                let default = f.get("default").ok_or_else(|| format!("field \"{key}\" has no default"))?;
                field.default = field
                    .sanitize(default)
                    .ok_or_else(|| format!("field \"{key}\": default does not match its type"))?;
            }
            fields.push(field);
        }
        Ok(Schema { raw, fields })
    }

    /// Keeps known keys with valid values, fills the rest with defaults, in schema order. Buttons have no value.
    pub fn normalize(&self, values: &Map<String, Value>) -> Map<String, Value> {
        self.fields
            .iter()
            .filter(|f| f.has_value())
            .map(|f| {
                let v = values.get(&f.key).and_then(|v| f.sanitize(v)).unwrap_or_else(|| f.default.clone());
                (f.key.clone(), v)
            })
            .collect()
    }

    /// Values after the module (re)declared this schema: like [`Self::normalize`], but display text takes the
    /// declared text.
    pub fn declare(&self, values: &Map<String, Value>) -> Map<String, Value> {
        let mut out = self.normalize(values);
        for f in self.fields.iter().filter(|f| matches!(f.kind, Kind::Display)) {
            out.insert(f.key.clone(), f.default.clone());
        }
        out
    }

    /// Overlays `patch` (partial object; invalid entries ignored) onto `current`, then normalizes. A patch from
    /// the user (`from_user`) cannot change display text.
    pub fn apply(&self, current: &Map<String, Value>, patch: &Map<String, Value>, from_user: bool) -> Map<String, Value> {
        let mut merged = self.normalize(current);
        for f in self.fields.iter().filter(|f| f.has_value() && (f.user_editable() || !from_user)) {
            if let Some(v) = patch.get(&f.key).and_then(|v| f.sanitize(v)) {
                merged.insert(f.key.clone(), v);
            }
        }
        merged
    }

    /// The values that are stored on disk and sent to the module: everything except display text.
    pub fn persisted(&self, values: &Map<String, Value>) -> Map<String, Value> {
        let display: HashSet<&str> =
            self.fields.iter().filter(|f| matches!(f.kind, Kind::Display)).map(|f| f.key.as_str()).collect();
        values.iter().filter(|(k, _)| !display.contains(k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn has_button(&self, key: &str) -> bool {
        self.fields.iter().any(|f| matches!(f.kind, Kind::Button) && f.key == key)
    }

    #[cfg(test)]
    pub fn defaults(&self) -> Map<String, Value> {
        self.normalize(&Map::new())
    }
}

impl Field {
    /// The value coerced into this field's type and range, or `None` when its type is wrong.
    fn sanitize(&self, v: &Value) -> Option<Value> {
        match &self.kind {
            Kind::Bool => v.as_bool().map(Value::Bool),
            Kind::Text | Kind::Display => v.as_str().map(|s| Value::String(s.to_owned())),
            Kind::Button => None,
            Kind::Choice(options) => v.as_str().filter(|s| options.iter().any(|o| o == s)).map(|s| Value::String(s.to_owned())),
            Kind::Int => {
                let n = match v.as_i64() {
                    Some(i) => i,
                    None => {
                        let f = v.as_f64()?;
                        if f.fract() != 0.0 || f.abs() > 9.0e15 {
                            return None;
                        }
                        f as i64
                    }
                };
                let lo = self.min.map_or(i64::MIN, |m| m.ceil() as i64);
                let hi = self.max.map_or(i64::MAX, |m| m.floor() as i64);
                Some(Value::from(n.clamp(lo, hi.max(lo))))
            }
            Kind::Float => {
                let f = v.as_f64()?;
                if !f.is_finite() {
                    return None;
                }
                let lo = self.min.unwrap_or(f64::NEG_INFINITY);
                let hi = self.max.unwrap_or(f64::INFINITY);
                Number::from_f64(f.clamp(lo, hi)).map(Value::Number)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SCHEMA: &str = r#"{ "fields": [
      { "key": "port", "label": "UDP port", "type": "int", "default": 11112, "min": 1, "max": 65535 },
      { "key": "smoothing", "label": "Smoothing", "type": "float", "default": 0.5, "min": 0, "max": 1, "step": 0.05 },
      { "key": "verbose", "label": "Verbose", "type": "bool", "default": false },
      { "key": "host", "label": "Host", "type": "string", "default": "" },
      { "key": "mode", "label": "Mode", "type": "choice", "default": "auto",
        "options": [ { "value": "auto", "label": "Automatic" }, { "value": "manual", "label": "Manual" } ] }
    ] }"#;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn defaults_fill_every_field() {
        let s = Schema::parse(SCHEMA).unwrap();
        assert_eq!(
            Value::Object(s.defaults()),
            json!({ "port": 11112, "smoothing": 0.5, "verbose": false, "host": "", "mode": "auto" })
        );
    }

    #[test]
    fn normalize_drops_unknown_and_wrong_types_clamps_and_fills() {
        let s = Schema::parse(SCHEMA).unwrap();
        let out = s.normalize(&obj(json!({
            "port": 99999,          // clamped to max
            "smoothing": "high",    // wrong type -> default
            "verbose": 1,           // not a bool -> default
            "mode": "turbo",        // not an option -> default
            "host": "10.0.0.5",
            "bogus": true           // unknown -> dropped
        })));
        assert_eq!(
            Value::Object(out),
            json!({ "port": 65535, "smoothing": 0.5, "verbose": false, "host": "10.0.0.5", "mode": "auto" })
        );
    }

    #[test]
    fn numbers_clamp_both_ways_and_ints_accept_integral_floats() {
        let s = Schema::parse(SCHEMA).unwrap();
        let out = s.normalize(&obj(json!({ "port": -5, "smoothing": 7 })));
        assert_eq!(out["port"], json!(1));
        assert_eq!(out["smoothing"], json!(1.0));
        let out = s.normalize(&obj(json!({ "port": 8080.0 })));
        assert_eq!(out["port"], json!(8080));
        let out = s.normalize(&obj(json!({ "port": 8080.5 })));
        assert_eq!(out["port"], json!(11112), "non-integral float is not an int");
    }

    #[test]
    fn apply_overlays_a_partial_patch_and_keeps_other_current_values() {
        let s = Schema::parse(SCHEMA).unwrap();
        let current = obj(json!({ "port": 2000, "host": "a", "mode": "manual" }));
        let out = s.apply(&current, &obj(json!({ "host": "b", "verbose": true, "port": "nope", "zzz": 1 })), true);
        assert_eq!(
            Value::Object(out),
            json!({ "port": 2000, "smoothing": 0.5, "verbose": true, "host": "b", "mode": "manual" })
        );
    }

    #[test]
    fn invalid_schemas_are_rejected() {
        let bad = [
            "not json",
            r#"{}"#,
            r#"{"fields":[{"key":"a","type":"int","default":1},{"key":"a","type":"bool","default":true}]}"#,
            r#"{"fields":[{"key":"a","default":1}]}"#,
            r#"{"fields":[{"key":"a","type":"int"}]}"#,
            r#"{"fields":[{"key":"a","type":"int","default":"x"}]}"#,
            r#"{"fields":[{"key":"a","type":"int","default":1,"min":5,"max":2}]}"#,
            r#"{"fields":[{"key":"m","type":"choice","default":"x","options":[]}]}"#,
            r#"{"fields":[{"key":"m","type":"choice","default":"z","options":[{"value":"x"}]}]}"#,
            r#"{"fields":[{"type":"bool","default":true}]}"#,
        ];
        for b in bad {
            assert!(Schema::parse(b).is_err(), "should reject {b}");
        }
        assert!(Schema::parse(r#"{"fields":[]}"#).is_ok());
        let future = Schema::parse(r#"{"fields":[{"key":"v","type":"vector","default":[1,2]},{"key":"b","type":"bool","default":true}]}"#).unwrap();
        assert_eq!(Value::Object(future.defaults()), json!({ "b": true }), "unknown field types are skipped");
    }

    #[test]
    fn out_of_range_default_is_clamped() {
        let s = Schema::parse(r#"{"fields":[{"key":"a","type":"int","default":500,"min":1,"max":10}]}"#).unwrap();
        assert_eq!(s.defaults()["a"], json!(10));
    }

    const PAIR: &str = r#"{ "fields": [
      { "key": "code", "label": "Pairing code", "type": "string", "default": "" },
      { "key": "pair", "label": "Pair by code", "type": "button" },
      { "key": "status", "label": "Status", "type": "text", "default": "Not paired" }
    ] }"#;

    #[test]
    fn display_text_is_module_owned_and_buttons_have_no_value() {
        let s = Schema::parse(PAIR).unwrap();
        assert!(s.has_button("pair") && !s.has_button("code") && !s.has_button("status"));
        assert_eq!(Value::Object(s.defaults()), json!({ "code": "", "status": "Not paired" }));
        let current = s.defaults();
        let user = s.apply(&current, &obj(json!({ "code": "1234", "status": "hacked", "pair": true })), true);
        assert_eq!(Value::Object(user.clone()), json!({ "code": "1234", "status": "Not paired" }));
        let module = s.apply(&user, &obj(json!({ "status": "Paired" })), false);
        assert_eq!(module["status"], json!("Paired"));
        assert_eq!(Value::Object(s.persisted(&module)), json!({ "code": "1234" }), "display text is never stored");
        // A new declaration brings the declared text back, but keeps the user's values.
        assert_eq!(Value::Object(s.declare(&module)), json!({ "code": "1234", "status": "Not paired" }));
    }
}
