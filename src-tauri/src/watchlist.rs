//! Persisted watchlist at `~/.cloud_burrito/watchlist.json`.
//!
//! Each thing is one specific item pinned to its own profile, account and
//! region. Shape validation lives in `validation::validate("watchlist_set")`.
//! Replacement is atomic through `Store`; the old `dashboard.json` is never
//! touched here.

use serde_json::{json, Value};

use crate::storage::{self, StorageError, Store};

pub(crate) const FILE: &str = "watchlist.json";

pub(crate) fn load(store: &Store) -> Result<Value, StorageError> {
    let Some(data) = store.read_json(FILE)? else {
        return Ok(storage::status(
            json!({"version": 1, "things": []}),
            "watchlist",
            "missing",
        ));
    };
    if !data.is_object() || data.get("version") != Some(&json!(1)) {
        return Err(StorageError::Invalid);
    }
    let things = data.get("things").ok_or(StorageError::Invalid)?;
    crate::validation::validate("watchlist_set", &json!({"things": things}))
        .map_err(|_| StorageError::Invalid)?;
    Ok(storage::status(
        json!({"version": 1, "things": things}),
        "watchlist",
        "loaded",
    ))
}

pub(crate) fn save(store: &Store, things: &Value) -> Result<Value, StorageError> {
    crate::validation::validate("watchlist_set", &json!({"things": things}))
        .map_err(|_| StorageError::Invalid)?;
    let out = json!({"version": 1, "things": things});
    store.write_json(FILE, &out)?;
    Ok(storage::status(out, "watchlist", "saved"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Store;
    use crate::test_support::TestDir;

    fn thing(id: &str) -> Value {
        json!({"id": id, "kind": "pipeline", "order": 0,
            "context": {"mode": "pinned", "profile": "synthetic-profile",
                "account_id": "acct-a-fixture", "region": "eu-west-1"},
            "inputs": {"pipeline_name": "synthetic-pipeline"}})
    }

    #[test]
    fn missing_saved_and_reloaded_are_distinct() {
        let tmp = TestDir::new();
        let store = Store::new(tmp.paths());
        let missing = load(&store).unwrap();
        assert_eq!(missing["_storage"]["status"], "missing");
        assert_eq!(missing["things"], json!([]));
        assert!(!tmp.paths().data_file(FILE).exists());
        let saved = save(&store, &json!([thing("t1")])).unwrap();
        assert_eq!(saved["_storage"]["status"], "saved");
        let loaded = load(&Store::new(tmp.paths())).unwrap();
        assert_eq!(loaded["_storage"]["status"], "loaded");
        assert_eq!(loaded["things"], json!([thing("t1")]));
    }

    #[test]
    fn corrupt_or_wrong_version_is_reported_and_kept() {
        let tmp = TestDir::new();
        let paths = tmp.paths();
        let store = Store::new(paths.clone());
        crate::file_privacy::ensure_directory(paths.data_dir()).unwrap();
        for bytes in [
            "{broken",
            "[]",
            "{\"version\":2,\"things\":[]}",
            "{\"version\":1}",
            "{\"version\":1,\"things\":[{\"id\":\"x\"}]}",
        ] {
            std::fs::write(paths.data_file(FILE), bytes).unwrap();
            assert_eq!(load(&store), Err(StorageError::Invalid));
            assert_eq!(
                std::fs::read_to_string(paths.data_file(FILE)).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn invalid_things_are_rejected_without_writing() {
        let tmp = TestDir::new();
        let store = Store::new(tmp.paths());
        let mut inherit = thing("t2");
        inherit["context"] = json!({"mode": "inherit"});
        let mut unknown = thing("t3");
        unknown["kind"] = json!("ec2-instances");
        let mut no_order = thing("t4");
        no_order.as_object_mut().unwrap().remove("order");
        let mut extra = thing("t5");
        extra["config"] = json!({});
        let mut bad_inputs = thing("t6");
        bad_inputs["inputs"] = json!("text");
        let too_many: Vec<Value> = (0..201).map(|n| thing(&format!("t{n}"))).collect();
        for things in [
            json!([thing("dup"), thing("dup")]),
            json!([inherit]),
            json!([unknown]),
            json!([no_order]),
            json!([extra]),
            json!([bad_inputs]),
            json!(too_many),
            json!("not-a-list"),
        ] {
            assert_eq!(save(&store, &things), Err(StorageError::Invalid));
            assert!(!tmp.paths().data_file(FILE).exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TestDir::new();
        save(&Store::new(tmp.paths()), &json!([thing("t1")])).unwrap();
        let mode = std::fs::metadata(tmp.paths().data_file(FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
