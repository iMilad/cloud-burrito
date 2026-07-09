//! Persistent user settings at `~/.cloud_burrito/settings.json`.
//!
//! Reads and writes a JSON file at startup and on every settings_set command
//! (originally ported from the Python sidecar). Unknown keys are dropped on
//! save; missing keys fall back to DEFAULTS on load; empty strings are treated
//! as "use the default" so a blank UI field doesn't pin a meaningless value.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::paths;

pub const ALLOWED_REGIONS: [&str; 2] = ["eu-west-1", "us-east-1"];

fn settings_path() -> PathBuf {
    paths::data_file("settings.json")
}

pub fn defaults() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("aws_config_path".into(), json!("~/.aws/config"));
    m.insert("sso_session_name".into(), json!(""));
    m.insert("default_profile".into(), json!(""));
    m.insert("default_region".into(), json!("eu-west-1"));
    m
}

pub fn load() -> Value {
    let mut merged = defaults();
    if let Ok(text) = fs::read_to_string(settings_path()) {
        if let Ok(Value::Object(data)) = serde_json::from_str::<Value>(&text) {
            for (k, v) in data {
                if merged.contains_key(&k) {
                    merged.insert(k, v);
                }
            }
        }
    }
    Value::Object(merged)
}

pub fn save(values: &Value) -> Value {
    let mut cleaned = defaults();
    if let Some(obj) = values.as_object() {
        for (k, v) in obj {
            if !cleaned.contains_key(k) {
                continue;
            }
            if let Some(s) = v.as_str() {
                let t = s.trim();
                if t.is_empty() {
                    continue; // blank -> keep default
                }
                cleaned.insert(k.clone(), json!(t));
            } else {
                cleaned.insert(k.clone(), v.clone());
            }
        }
    }
    let out = Value::Object(cleaned);
    let p = settings_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&out) {
        let _ = fs::write(&p, text);
    }
    out
}

/// Read a string field from a settings value, defaulting to "".
pub fn get_str(settings: &Value, key: &str) -> String {
    settings
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_drops_unknown_keys_and_blanks() {
        let _g = crate::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _prev_home = std::env::var_os("HOME");
        let tmp = std::env::temp_dir().join(format!("acc-settings-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        std::env::set_var("HOME", &tmp);

        let saved = save(&json!({
            "default_region": "us-east-1",
            "sso_session_name": "   ",          // blank -> default ""
            "bogus": "nope",                      // unknown -> dropped
        }));
        assert_eq!(saved["default_region"], json!("us-east-1"));
        assert_eq!(saved["sso_session_name"], json!(""));
        assert!(saved.get("bogus").is_none());

        let loaded = load();
        assert_eq!(loaded["default_region"], json!("us-east-1"));
        match _prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}
