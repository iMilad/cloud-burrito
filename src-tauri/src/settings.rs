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
    m.insert("theme".into(), json!("dark"));
    m.insert("audit_retention".into(), json!("preserve"));
    m
}

pub fn allowed_region(region: &str) -> bool {
    ALLOWED_REGIONS.contains(&region)
}

/// The same normalization applies to a saved document and submitted fields.
/// Semantic errors remain visible on read so a preference can be repaired.
fn normal_form(values: &Value) -> Result<Value, StorageError> {
    let mut cleaned = defaults();
    let obj = values.as_object().ok_or(StorageError::Invalid)?;
    for (k, v) in obj {
        if !cleaned.contains_key(k) {
            continue;
        }
        if k == "audit_retention" {
            // Retention is an explicit choice; only an absent legacy key defaults.
            cleaned.insert(k.clone(), v.clone());
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
    crate::validation::settings_shape(&out).map_err(|_| StorageError::Invalid)?;
    Ok(out)
}

pub(crate) fn with_metadata(mut response: Value, fields: &Value) -> Value {
    response["_settings"] = json!({
        "defaults": defaults(),
        "allowed_regions": ALLOWED_REGIONS,
        "field_errors": crate::validation::settings_field_errors(fields),
    });
    response
}

pub(crate) fn load(store: &Store) -> Result<Value, StorageError> {
    let (value, status) = match store.read_json("settings.json")? {
        Some(data) => (normal_form(&data)?, "loaded"),
        None => (Value::Object(defaults()), "missing"),
    };
    Ok(with_metadata(
        storage::status(value.clone(), "settings", status),
        &value,
    ))
}

pub(crate) fn save(store: &Store, values: &Value) -> Result<Value, StorageError> {
    let out = normal_form(values)?;
    crate::validation::validate("settings_set", &out).map_err(|_| StorageError::Invalid)?;
    store.write_json("settings.json", &out)?;
    Ok(with_metadata(
        storage::status(out.clone(), "settings", "saved"),
        &out,
    ))
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
        let values = json!({"aws_config_path":"~/synthetic/config.ini", "sso_session_name":"synthetic-session", "default_profile":"synthetic-profile", "default_region":"us-east-1", "theme":"light", "audit_retention":"preserve"});
        let saved = save(&store, &values).unwrap();
        assert_eq!(saved["_storage"]["status"], "saved");
        let mut loaded = load(&Store::new(paths.clone())).unwrap();
        assert_eq!(loaded["_storage"]["status"], "loaded");
        loaded.as_object_mut().unwrap().remove("_storage");
        loaded.as_object_mut().unwrap().remove("_settings");
        assert_eq!(loaded, values);
        let persisted: Value =
            serde_json::from_slice(&std::fs::read(paths.data_file("settings.json")).unwrap())
                .unwrap();
        assert_eq!(persisted, values);
    }

    #[test]
    fn defaults_metadata_and_blank_normalization_are_the_same_after_reopen() {
        let tmp = TestDir::new();
        let store = Store::new(tmp.paths());
        let first = load(&store).unwrap();
        assert_eq!(first["_settings"]["defaults"], Value::Object(defaults()));
        assert_eq!(
            first["_settings"]["allowed_regions"],
            json!(ALLOWED_REGIONS)
        );
        assert_eq!(first["_settings"]["field_errors"], json!({}));
        let blanks = json!({"aws_config_path":"  ", "sso_session_name":"\t", "default_profile":" ", "default_region":" ", "theme":" "});
        let saved = save(&store, &blanks).unwrap();
        for (key, value) in defaults() {
            assert_eq!(saved[&key], value);
        }
        // Existing hand-edited blank strings use the same normal form on read.
        store.write_json("settings.json", &blanks).unwrap();
        let reopened = load(&Store::new(tmp.paths())).unwrap();
        for (key, value) in defaults() {
            assert_eq!(reopened[&key], value);
        }
    }

    #[test]
    fn unsupported_saved_region_remains_visible_and_can_be_explicitly_corrected() {
        let tmp = TestDir::new();
        let store = Store::new(tmp.paths());
        let saved = json!({"default_region":"ap-south-1", "theme":"light"});
        store.write_json("settings.json", &saved).unwrap();
        let loaded = load(&store).unwrap();
        assert_eq!(loaded["default_region"], "ap-south-1");
        assert_eq!(loaded["theme"], "light");
        assert_eq!(
            loaded["_settings"]["field_errors"]["default_region"],
            "Choose a supported default region"
        );
        assert_eq!(store.read_json("settings.json").unwrap().unwrap(), saved);
        assert_eq!(save(&store, &saved), Err(StorageError::Invalid));
        assert_eq!(store.read_json("settings.json").unwrap().unwrap(), saved);
        let corrected = save(
            &store,
            &json!({"default_region":"us-east-1", "theme":"light"}),
        )
        .unwrap();
        assert_eq!(corrected["_settings"]["field_errors"], json!({}));
        assert_eq!(load(&Store::new(tmp.paths())).unwrap()["theme"], "light");
        let persisted = store.read_json("settings.json").unwrap().unwrap();
        assert!(persisted.get("_settings").is_none());
        assert!(persisted.get("_storage").is_none());
    }
}
