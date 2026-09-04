//! Persistent user settings at `~/.cloud_burrito/settings.json`.
//!
//! Reads and writes a JSON file at startup and on every settings_set command
//! (originally ported from the Python sidecar). Unknown keys are dropped on
//! save; missing keys fall back to DEFAULTS on load; empty strings are treated
//! as "use the default" so a blank UI field doesn't pin a meaningless value.

use serde_json::{json, Map, Value};

use crate::storage::{self, StorageError, Store};

pub const ALLOWED_REGIONS: [&str; 2] = ["eu-west-1", "us-east-1"];

pub fn defaults() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("aws_config_path".into(), json!("~/.aws/config"));
    m.insert("sso_session_name".into(), json!(""));
    m.insert("default_profile".into(), json!(""));
    m.insert("default_region".into(), json!("eu-west-1"));
    m
}

pub(crate) fn load(store: &Store) -> Result<Value, StorageError> {
    let mut merged = defaults();
    let Some(data) = store.read_json("settings.json")? else {
        return Ok(storage::status(
            Value::Object(merged),
            "settings",
            "missing",
        ));
    };
    let data = data.as_object().ok_or(StorageError::Invalid)?;
    for (k, v) in data {
        if merged.contains_key(k) {
            merged.insert(k.clone(), v.clone());
        }
    }
    let value = Value::Object(merged);
    crate::validation::validate("settings_set", &value).map_err(|_| StorageError::Invalid)?;
    Ok(storage::status(value, "settings", "loaded"))
}

pub(crate) fn save(store: &Store, values: &Value) -> Result<Value, StorageError> {
    let mut cleaned = defaults();
    let obj = values.as_object().ok_or(StorageError::Invalid)?;
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
    let out = Value::Object(cleaned);
    crate::validation::validate("settings_set", &out).map_err(|_| StorageError::Invalid)?;
    store.write_json("settings.json", &out)?;
    Ok(storage::status(out, "settings", "saved"))
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
    use crate::test_support::TestDir;

    #[test]
    fn save_drops_unknown_keys_and_blanks() {
        let tmp = TestDir::new();
        let store = Store::new(tmp.paths());

        let saved = save(
            &store,
            &json!({
                "default_region": "us-east-1",
                "sso_session_name": "   ",          // blank -> default ""
                "bogus": "nope",                      // unknown -> dropped
            }),
        )
        .unwrap();
        assert_eq!(saved["default_region"], json!("us-east-1"));
        assert_eq!(saved["sso_session_name"], json!(""));
        assert!(saved.get("bogus").is_none());

        let loaded = load(&store).unwrap();
        assert_eq!(loaded["default_region"], json!("us-east-1"));
    }

    #[test]
    fn missing_and_corrupt_settings_are_distinct_without_writing() {
        let tmp = TestDir::new();
        let paths = tmp.paths();
        let store = Store::new(paths.clone());
        assert_eq!(load(&store).unwrap()["_storage"]["status"], "missing");
        assert!(!paths.data_file("settings.json").exists());
        std::fs::create_dir_all(paths.data_file("settings.json").parent().unwrap()).unwrap();
        for bytes in ["{broken", "[]", "{\"default_profile\":12}"] {
            std::fs::write(paths.data_file("settings.json"), bytes).unwrap();
            assert_eq!(load(&store), Err(StorageError::Invalid));
            assert_eq!(
                std::fs::read_to_string(paths.data_file("settings.json")).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn supported_values_survive_reopen_without_persisting_status() {
        let tmp = TestDir::new();
        let paths = tmp.paths();
        let store = Store::new(paths.clone());
        let values = json!({"aws_config_path":"~/synthetic/config.ini", "sso_session_name":"synthetic-session", "default_profile":"synthetic-profile", "default_region":"us-east-1"});
        let saved = save(&store, &values).unwrap();
        assert_eq!(saved["_storage"]["status"], "saved");
        let mut loaded = load(&Store::new(paths.clone())).unwrap();
        assert_eq!(loaded["_storage"]["status"], "loaded");
        loaded.as_object_mut().unwrap().remove("_storage");
        assert_eq!(loaded, values);
        let persisted: Value =
            serde_json::from_slice(&std::fs::read(paths.data_file("settings.json")).unwrap())
                .unwrap();
        assert_eq!(persisted, values);
    }
}
