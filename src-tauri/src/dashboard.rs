//! Persisted dashboard layout at `~/.cloud_burrito/dashboard.json`.
//!
//! Stores an array of tile descriptors keyed by unique tile `id`, optional
//! widget type, `x,y,w,h` geometry, and an opaque `config` dict that the
//! frontend owns (context, header_color, inputs). Replacement is atomic; native
//! crash/power-loss behavior remains part of platform acceptance.

use serde_json::{json, Map, Value};

use crate::storage::{self, StorageError, Store};

pub(crate) fn load(store: &Store) -> Result<Value, StorageError> {
    let default = json!({"version": 1, "tiles": []});
    let Some(data) = store.read_json("dashboard.json")? else {
        return Ok(storage::status(default, "dashboard", "missing"));
    };
    if !data.is_object() || data.get("version").is_some_and(|version| version != 1) {
        return Err(StorageError::Invalid);
    }
    let tiles = data.get("tiles").ok_or(StorageError::Invalid)?;
    crate::validation::validate("dashboard_set", &json!({"tiles":tiles}))
        .map_err(|_| StorageError::Invalid)?;
    Ok(storage::status(
        json!({"version":1,"tiles":tiles}),
        "dashboard",
        "loaded",
    ))
}

pub(crate) fn save(store: &Store, tiles: &Value) -> Result<Value, StorageError> {
    let mut cleaned: Vec<Value> = Vec::new();
    let arr = tiles.as_array().ok_or(StorageError::Invalid)?;
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
    let out = json!({"version": 1, "tiles": cleaned});
    crate::validation::validate("dashboard_set", &json!({"tiles":out["tiles"]}))
        .map_err(|_| StorageError::Invalid)?;
    store.write_json("dashboard.json", &out)?;
    Ok(storage::status(out, "dashboard", "saved"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn save_filters_to_known_fields() {
        let tmp = TestDir::new();
        let store = Store::new(tmp.paths());

        let out = save(
            &store,
            &json!([
                {"id": "t1", "widget": "pipeline-runs", "x": 0, "y": 0, "w": 4, "h": 3, "config": {"header_color": "blue"}, "junk": 1},
                {"x": 1, "y": 1},                 // no id -> dropped
                "not-an-object"                    // dropped
            ]),
        ).unwrap();
        let tiles = out["tiles"].as_array().unwrap();
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0]["id"], json!("t1"));
        assert_eq!(tiles[0]["widget"], json!("pipeline-runs"));
        assert!(tiles[0].get("junk").is_none());
        assert_eq!(tiles[0]["config"]["header_color"], json!("blue"));
        assert_eq!(load(&store).unwrap()["tiles"], out["tiles"]);
    }

    #[test]
    fn empty_dashboard_is_distinct_from_missing_and_corrupt() {
        let tmp = TestDir::new();
        let paths = tmp.paths();
        let store = Store::new(paths.clone());
        assert_eq!(load(&store).unwrap()["_storage"]["status"], "missing");
        assert!(!paths.data_file("dashboard.json").exists());
        save(&store, &json!([])).unwrap();
        let loaded = load(&store).unwrap();
        assert_eq!(loaded["_storage"]["status"], "loaded");
        assert_eq!(loaded["tiles"], json!([]));
        for bytes in [
            "{broken",
            "[]",
            "{\"version\":2,\"tiles\":[]}",
            "{\"tiles\":[{\"id\":false}]}",
        ] {
            std::fs::write(paths.data_file("dashboard.json"), bytes).unwrap();
            assert_eq!(load(&store), Err(StorageError::Invalid));
            assert_eq!(
                std::fs::read_to_string(paths.data_file("dashboard.json")).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn layout_and_all_supported_pin_shapes_survive_reopen() {
        let tmp = TestDir::new();
        let paths = tmp.paths();
        let store = Store::new(paths.clone());
        let identity = json!({"mode":"pinned", "profile":"synthetic-profile", "account_id":"acct-a-fixture", "region":"eu-west-1"});
        let pin = json!({"id":"synthetic-pin", "profile":"synthetic-profile", "account_id":"acct-a-fixture", "region":"eu-west-1"});
        let mut pipeline = pin.clone();
        pipeline["pipeline_name"] = json!("synthetic-pipeline");
        let mut cli = pin;
        cli["command"] = json!("aws cloudformation list-stacks");
        let tiles = json!([
            {"id":"pipeline", "widget":"pipeline-runs", "x":0,"y":0,"w":6,"h":4,
                "config":{"context":identity, "header_color":"purple", "header_style":"gradient", "collapsed":true, "expanded_height":4, "inputs":{"pinned_pipelines":[pipeline]}}},
            {"id":"cli", "widget":"aws-cli", "x":6,"y":0,"w":6,"h":4,
                "config":{"inputs":{"command":"aws cloudformation list-stacks", "pinned_cli_commands":[cli]}}}
        ]);
        save(&store, &tiles).unwrap();
        assert_eq!(load(&Store::new(paths.clone())).unwrap()["tiles"], tiles);
        let persisted: Value =
            serde_json::from_slice(&std::fs::read(paths.data_file("dashboard.json")).unwrap())
                .unwrap();
        assert_eq!(persisted, json!({"version":1,"tiles":tiles}));
    }
}
