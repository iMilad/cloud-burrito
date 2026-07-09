//! Persisted dashboard layout at `~/.cloud_burrito/dashboard.json`.
//!
//! Stores an array of tile descriptors keyed by unique tile `id`, optional
//! widget type, `x,y,w,h` geometry, and an opaque `config` dict that the
//! frontend owns (context, header_color, inputs). Atomic write (tmp + rename)
//! so a quit mid-write never leaves a half file (originally ported from the
//! Python sidecar).

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

fn layout_path() -> PathBuf {
    crate::paths::data_file("dashboard.json")
}

pub fn load() -> Value {
    let default = json!({"version": 1, "tiles": []});
    let text = match fs::read_to_string(layout_path()) {
        Ok(t) => t,
        Err(_) => return default,
    };
    let data: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return default,
    };
    let tiles_ok = data.get("tiles").map(Value::is_array).unwrap_or(false);
    if !data.is_object() || !tiles_ok {
        return default;
    }
    json!({"version": 1, "tiles": data.get("tiles").cloned().unwrap_or_else(|| json!([]))})
}

pub fn save(tiles: &Value) -> Value {
    let mut cleaned: Vec<Value> = Vec::new();
    if let Some(arr) = tiles.as_array() {
        for t in arr {
            let obj = match t.as_object() {
                Some(o) => o,
                None => continue,
            };
            let mut keep = Map::new();
            for k in ["id", "widget", "x", "y", "w", "h"] {
                if let Some(v) = obj.get(k) {
                    keep.insert(k.to_string(), v.clone());
                }
            }
            if !keep.contains_key("id") {
                continue;
            }
            if let Some(cfg) = obj.get("config") {
                if cfg.is_object() {
                    keep.insert("config".into(), cfg.clone());
                }
            }
            cleaned.push(Value::Object(keep));
        }
    }
    let out = json!({"version": 1, "tiles": cleaned});
    let p = layout_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&out) {
        let tmp = p.with_extension("json.tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, &p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_filters_to_known_fields() {
        let _g = crate::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _prev_home = std::env::var_os("HOME");
        let tmp = std::env::temp_dir().join(format!("acc-dash-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        std::env::set_var("HOME", &tmp);

        let out = save(&json!([
            {"id": "t1", "widget": "pipeline-runs", "x": 0, "y": 0, "w": 4, "h": 3, "config": {"header_color": "blue"}, "junk": 1},
            {"x": 1, "y": 1},                 // no id -> dropped
            "not-an-object"                    // dropped
        ]));
        let tiles = out["tiles"].as_array().unwrap();
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0]["id"], json!("t1"));
        assert_eq!(tiles[0]["widget"], json!("pipeline-runs"));
        assert!(tiles[0].get("junk").is_none());
        assert_eq!(tiles[0]["config"]["header_color"], json!("blue"));
        match _prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}
