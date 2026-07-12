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

/// Serializes concurrent appends so interleaved widget fan-out never produces
/// a half-written line.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn log_path() -> PathBuf {
    crate::paths::data_file("audit.log")
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
pub fn append(entry: Value) {
    let mut obj: Map<String, Value> = match entry {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("value".into(), other);
            m
        }
    };
    obj.insert("ts".into(), json!(now_epoch()));
    let line = match serde_json::to_string(&Value::Object(obj)) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _g = WRITE_LOCK.lock();
    let p = log_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{line}");
    }
}

/// Return the last `limit` well-formed entries, oldest-first. Malformed lines
/// are skipped — a corrupted log never propagates to the UI.
pub fn tail(limit: usize) -> Vec<Value> {
    let p = log_path();
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

    #[test]
    fn append_injects_ts_and_tail_roundtrips() {
        let _g = crate::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _prev_home = std::env::var_os("HOME");
        // Point at a temp HOME so we don't touch the real audit log.
        let tmp = std::env::temp_dir().join(format!("acc-audit-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        std::env::set_var("HOME", &tmp);

        append(json!({"kind": "lifecycle", "event": "unit_test"}));
        let entries = tail(50);
        assert!(!entries.is_empty());
        let last = entries.last().unwrap();
        assert_eq!(last["kind"], json!("lifecycle"));
        assert_eq!(last["event"], json!("unit_test"));
        assert!(last["ts"].as_f64().unwrap() > 0.0);
        match _prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}
