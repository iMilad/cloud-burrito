//! Append-only JSONL audit log of AWS API calls and lifecycle events
//! (originally ported from the Python sidecar). One JSON object per line at
//! `~/.cloud_burrito/audit.log`. Two kinds dominate:
//! - `{"kind":"lifecycle","event":...}` — set-account attempts/results.
//! - `{"kind":"aws","service":...,"operation":...,"account_id":...,"region":...}`
//!   — one entry per AWS API call (the read-only guard would tag a blocked
//!   call `"aws-blocked"`, but in the pure-Rust client we only ever issue
//!   read operations, so that never fires in practice).
//!
//! Raw request params are NEVER logged — they can contain account IDs / PII.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::paths::AppPaths;

/// Serializes concurrent appends so interleaved widget fan-out never produces
/// a half-written line.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn log_path(paths: &AppPaths) -> PathBuf {
    paths.data_file("audit.log")
}

/// Epoch seconds as a float, matching Python's `time.time()` so the frontend's
/// `new Date(entry.ts * 1000)` keeps working unchanged.
pub fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Append one entry. A `ts` field is injected; any `ts` in `entry` is overwritten.
pub fn append(paths: &AppPaths, entry: Value, timestamp: f64) {
    let mut obj: Map<String, Value> = match entry {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("value".into(), other);
            m
        }
    };
    obj.insert("ts".into(), json!(timestamp));
    let line = match serde_json::to_string(&Value::Object(obj)) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _g = WRITE_LOCK.lock();
    let p = log_path(paths);
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{line}");
    }
}

/// Return the last `limit` well-formed entries, oldest-first. Malformed lines
/// are skipped — a corrupted log never propagates to the UI.
pub fn tail(paths: &AppPaths, limit: usize) -> Vec<Value> {
    let p = log_path(paths);
    let file = match fs::File::open(&p) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    let start = lines.len().saturating_sub(limit);
    let mut out = Vec::new();
    for line in &lines[start..] {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(t) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn append_injects_ts_and_tail_roundtrips() {
        let tmp = TestDir::new();
        let paths = tmp.paths();

        append(
            &paths,
            json!({"kind": "lifecycle", "event": "unit_test", "ts": 1.0}),
            42.5,
        );
        let entries = tail(&paths, 50);
        assert!(!entries.is_empty());
        let last = entries.last().unwrap();
        assert_eq!(last["kind"], json!("lifecycle"));
        assert_eq!(last["event"], json!("unit_test"));
        assert_eq!(last["ts"], json!(42.5));
    }
}
