//! Local application-intent and capability-decision diagnostics.
//!
//! These are not SDK wire-call counts: a preflight can cover pagination,
//! retries or provider work. Only explicitly selected structured fields are
//! persisted. Raw inputs, resource payloads, errors and configuration are not.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::paths::AppPaths;

static WRITE_LOCK: Mutex<()> = Mutex::new(());
pub const MAX_TAIL_ENTRIES: usize = 1000;
// Bound retained memory even for old, malformed or externally edited logs.
const MAX_LINE_BYTES: usize = 16 * 1024;

fn log_path(paths: &AppPaths) -> PathBuf {
    paths.data_file("audit.log")
}

pub fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn identifier(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    })
}

/// An allowlist, not recursive redaction. Arbitrary nested fields are dropped
/// instead of trying to recognize every possible credential spelling.
fn sanitized(entry: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for key in [
        "kind",
        "event",
        "command",
        "widget",
        "service",
        "operation",
        "error_type",
        "request_id",
    ] {
        if let Some(value) = entry.get(key).and_then(identifier) {
            out.insert(key.into(), json!(value));
        }
    }
    let preflight = matches!(entry["kind"].as_str(), Some("aws" | "aws-blocked"));
    out.insert(
        "scope".into(),
        json!(if preflight {
            "capability_preflight"
        } else {
            "application"
        }),
    );
    if !preflight {
        for key in ["account_id", "region"] {
            if let Some(value) = entry.get(key).and_then(identifier) {
                out.insert(key.into(), json!(value));
            }
        }
    }
    if let Some(status @ ("stopped" | "not_confirmed" | "denied" | "failed" | "not_attempted")) =
        entry.get("cleanup_status").and_then(Value::as_str)
    {
        out.insert("cleanup_status".into(), json!(status));
        out.insert("event".into(), json!("query_cleanup"));
    }
    for key in [
        "attempt",
        "failed_count",
        "queried",
        "succeeded",
        "failed",
        "timed_out",
        "remote_status_unknown",
    ] {
        if let Some(value) = entry.get(key).and_then(Value::as_u64) {
            out.insert(key.into(), json!(value));
        }
    }
    out
}

/// Failure is explicit and deliberately carries no OS path or error payload.
pub fn append(paths: &AppPaths, entry: Value, timestamp: f64) -> Result<(), ()> {
    let mut obj = sanitized(&entry);
    obj.insert("ts".into(), json!(timestamp));
    let line = serde_json::to_string(&obj).map_err(|_| ())?;
    let _guard = WRITE_LOCK.lock();
    let path = log_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| ())?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|_| ())?;
    writeln!(file, "{line}").map_err(|_| ())
}

/// Scan history with bounded retained entries/line bytes. Disk scan time and
/// rotation/retention remain P3 work. Missing history is a normal first run;
/// an unreadable history is an explicit failure, never a fabricated empty log.
pub fn try_tail(paths: &AppPaths, limit: usize) -> Result<Vec<Value>, ()> {
    let limit = limit.min(MAX_TAIL_ENTRIES);
    if limit == 0 {
        return Ok(Vec::new());
    }
    let file = match fs::File::open(log_path(paths)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(()),
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut oversized = false;
    let mut entries = VecDeque::new();
    loop {
        let buf = reader.fill_buf().map_err(|_| ())?;
        if buf.is_empty() {
            break;
        }
        let count = buf
            .iter()
            .position(|b| *b == b'\n')
            .map_or(buf.len(), |p| p + 1);
        let ends_line = buf[count - 1] == b'\n';
        if !oversized && line.len() + count <= MAX_LINE_BYTES {
            line.extend_from_slice(&buf[..count]);
        } else {
            oversized = true;
            line.clear();
        }
        reader.consume(count);
        if ends_line {
            if !oversized {
                retain_line(&line, &mut entries, limit);
            }
            line.clear();
            oversized = false;
        }
    }
    if !oversized && !line.is_empty() {
        retain_line(&line, &mut entries, limit);
    }
    Ok(entries.into_iter().collect())
}

fn retain_line(line: &[u8], entries: &mut VecDeque<Value>, limit: usize) {
    if let Ok(value) = serde_json::from_slice::<Value>(line) {
        if !value.is_object() {
            return;
        }
        let mut safe = sanitized(&value);
        if let Some(ts) = value.get("ts").and_then(Value::as_f64) {
            safe.insert("ts".into(), json!(ts));
        }
        if entries.len() == limit {
            entries.pop_front();
        }
        entries.push_back(Value::Object(safe));
    }
}

#[cfg(test)]
pub fn tail(paths: &AppPaths, limit: usize) -> Vec<Value> {
    try_tail(paths, limit).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn structured_events_roundtrip_without_private_payloads() {
        let dir = TestDir::new();
        let paths = dir.paths();
        let marker = "synthetic-private-marker";
        append(
            &paths,
            json!({"kind":"request", "event":"failed", "request_id":"req-1",
            "account_id":"acct-fixture", "region":"us-east-1", "error_type":"ClientError",
            "error":marker, "message":marker, "reason":marker, "command_args":[marker],
            "credentials":{"value":marker}, "config_path":marker, "query":marker, "ts":1}),
            42.5,
        )
        .unwrap();
        let persisted = fs::read_to_string(log_path(&paths)).unwrap();
        assert!(!persisted.contains(marker));
        let entries = try_tail(&paths, 10).unwrap();
        assert_eq!(entries[0]["event"], "failed");
        assert_eq!(entries[0]["scope"], "application");
        assert_eq!(entries[0]["ts"], 42.5);
        assert_eq!(entries[0]["account_id"], "acct-fixture");
    }

    #[test]
    fn unreadable_or_unwritable_history_is_not_empty_success() {
        let dir = TestDir::new();
        let paths = dir.paths();
        assert!(try_tail(&paths, 10).unwrap().is_empty());
        fs::create_dir_all(log_path(&paths)).unwrap();
        assert!(append(&paths, json!({"event":"started"}), 1.0).is_err());
        assert!(try_tail(&paths, 10).is_err());
    }

    #[test]
    fn tail_caps_entries_skips_oversized_lines_and_sanitizes_legacy_rows() {
        let dir = TestDir::new();
        let paths = dir.paths();
        fs::create_dir_all(log_path(&paths).parent().unwrap()).unwrap();
        let mut data = format!("{}\n", "x".repeat(MAX_LINE_BYTES * 2));
        for i in 0..1100 {
            data.push_str(&format!("{{\"kind\":\"aws\",\"operation\":\"ListStacks\",\"account_id\":\"unverified\",\"error\":\"synthetic-private-marker\",\"ts\":{i}}}\n"));
        }
        fs::write(log_path(&paths), data).unwrap();
        let entries = try_tail(&paths, usize::MAX).unwrap();
        assert_eq!(entries.len(), MAX_TAIL_ENTRIES);
        assert_eq!(entries[0]["ts"], 100.0);
        assert_eq!(entries[0]["scope"], "capability_preflight");
        assert!(entries[0].get("account_id").is_none());
        assert!(!serde_json::to_string(&entries)
            .unwrap()
            .contains("synthetic-private-marker"));
    }
}
