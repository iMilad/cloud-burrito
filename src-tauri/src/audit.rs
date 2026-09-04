//! Local application-intent and capability-decision diagnostics.
//!
//! These are not SDK wire-call counts: a preflight can cover pagination,
//! retries or provider work. Only explicitly selected structured fields are
//! persisted. Raw inputs, resource payloads, errors and configuration are not.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, Metadata, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::paths::AppPaths;

static WRITE_LOCK: Mutex<()> = Mutex::new(());
pub const MAX_TAIL_ENTRIES: usize = 1000;
// Bound retained memory even for old, malformed or externally edited logs.
pub(crate) const MAX_LINE_BYTES: usize = 16 * 1024;

pub(crate) fn log_path(paths: &AppPaths) -> PathBuf {
    paths.data_file("audit.log")
}

/// Final-component protection only: AppPaths parents are trusted. These
/// platform OpenOptions flags introduce no native dependency.
pub(crate) fn no_follow_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NOFOLLOW: i32 = 0x0000_0100;
        options.custom_flags(O_NOFOLLOW);
    }
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64")
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NOFOLLOW: i32 = 0x0002_0000;
        options.custom_flags(O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
}

pub(crate) fn file_identity(metadata: &Metadata) -> u64 {
    let mut hash = DefaultHasher::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.dev().hash(&mut hash);
        metadata.ino().hash(&mut hash);
    }
    // Creation time is the portable fallback; boundary fingerprints still
    // validate the continuation when filesystem identity APIs are unavailable.
    #[cfg(not(unix))]
    {
        metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .hash(&mut hash);
    }
    hash.finish()
}

pub(crate) fn regular_file(metadata: &Metadata) -> bool {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    true
}

/// Compare the inspected filename, opened handle and current filename before
/// content I/O. New-file callers must set create_new, so a raced-in path fails.
pub(crate) fn checked_open(
    path: &Path,
    inspected: Option<&Metadata>,
    options: &OpenOptions,
) -> Result<(File, Metadata), ()> {
    if inspected.is_some_and(|metadata| !regular_file(metadata)) {
        return Err(());
    }
    let file = options.open(path).map_err(|_| ())?;
    let opened = file.metadata().map_err(|_| ())?;
    let current = fs::symlink_metadata(path).map_err(|_| ())?;
    if !regular_file(&opened)
        || !regular_file(&current)
        || inspected.is_some_and(|metadata| file_identity(metadata) != file_identity(&opened))
        || file_identity(&current) != file_identity(&opened)
    {
        return Err(());
    }
    Ok((file, opened))
}

fn existing_metadata(path: &Path) -> Result<Option<Metadata>, ()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if regular_file(&metadata) => Ok(Some(metadata)),
        Ok(_) => Err(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

fn same_current_file(path: &Path, opened: &Metadata) -> bool {
    fs::symlink_metadata(path).is_ok_and(|current| {
        regular_file(&current) && file_identity(&current) == file_identity(opened)
    })
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
pub(crate) fn sanitized(entry: &Value) -> Map<String, Value> {
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
    if let Some(
        status @ ("stopped" | "not_confirmed" | "denied" | "failed" | "not_attempted" | "unknown"
        | "not_needed"),
    ) = entry.get("cleanup_status").and_then(Value::as_str)
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

pub(crate) const RETENTION_FILES: usize = 5;
pub(crate) const RETENTION_FILE_BYTES: u64 = 10 * 1024 * 1024;
static NEXT_PRESERVATION: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
static PRESERVE_MODE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetentionMode {
    Preserve,
    Bounded,
}
impl RetentionMode {
    pub fn as_str(self) -> &'static str {
        if self == Self::Bounded {
            "bounded"
        } else {
            "preserve"
        }
    }
}

pub(crate) fn encoded(entry: &Value, timestamp: f64) -> Result<Vec<u8>, ()> {
    let mut object = sanitized(entry);
    object.insert("ts".into(), json!(timestamp));
    let mut line = serde_json::to_vec(&object).map_err(|_| ())?;
    line.push(b'\n');
    if line.len() > MAX_LINE_BYTES {
        return Err(());
    }
    Ok(line)
}

/// Synchronous compatibility path for isolated tests and the baseline replay.
/// Production Runtime uses the ordered bounded writer queue.
#[cfg(test)]
pub fn append(paths: &AppPaths, entry: Value, timestamp: f64) -> Result<(), ()> {
    append_record(paths, &encoded(&entry, timestamp)?, &PRESERVE_MODE)
}

pub(crate) fn append_record(
    paths: &AppPaths,
    line: &[u8],
    retention: &AtomicBool,
) -> Result<(), ()> {
    if line.is_empty() || line.len() > MAX_LINE_BYTES || line.last() != Some(&b'\n') {
        return Err(());
    }
    let _guard = WRITE_LOCK.lock();
    let path = log_path(paths);
    fs::create_dir_all(path.parent().ok_or(())?).map_err(|_| ())?;
    let sizes = history_sizes(paths)?;
    if retention.load(Ordering::SeqCst) {
        if sizes.iter().any(|(_, size)| *size > RETENTION_FILE_BYTES) {
            return Err(());
        }
        let active = sizes
            .iter()
            .find(|(name, _)| name == "audit.log")
            .map_or(0, |(_, size)| *size);
        if active + line.len() as u64 > RETENTION_FILE_BYTES {
            rotate(paths)?;
        }
    }
    let inspected = existing_metadata(&path)?;
    let mut options = no_follow_options();
    options.append(true);
    if inspected.is_none() {
        options.create_new(true);
    }
    let (mut file, opened) = checked_open(&path, inspected.as_ref(), &options)?;
    if retention.load(Ordering::SeqCst)
        && opened.len().saturating_add(line.len() as u64) > RETENTION_FILE_BYTES
    {
        return Err(());
    }
    file.write_all(line).map_err(|_| ())?;
    if !same_current_file(&path, &opened) {
        return Err(());
    }
    Ok(())
}

/// Flush the current file under the same lock as append, rotation and preserve.
/// A write-capable existing handle is needed for FlushFileBuffers on Windows.
pub(crate) fn sync_history(paths: &AppPaths) -> Result<(), ()> {
    let _guard = WRITE_LOCK.lock();
    let path = log_path(paths);
    let Some(inspected) = existing_metadata(&path)? else {
        return Ok(());
    };
    let mut options = no_follow_options();
    options.write(true);
    let (file, opened) = checked_open(&path, Some(&inspected), &options)?;
    file.sync_data().map_err(|_| ())?;
    if !same_current_file(&path, &opened) {
        return Err(());
    }
    Ok(())
}

fn file_names() -> Vec<String> {
    std::iter::once("audit.log".to_string())
        .chain((1..RETENTION_FILES).map(|index| format!("audit.{index}")))
        .collect()
}

fn history_sizes(paths: &AppPaths) -> Result<Vec<(String, u64)>, ()> {
    let mut sizes = Vec::new();
    for name in file_names() {
        match fs::symlink_metadata(paths.data_file(&name)) {
            Ok(metadata) if regular_file(&metadata) => sizes.push((name, metadata.len())),
            Ok(_) => return Err(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(()),
        }
    }
    Ok(sizes)
}

fn rotate(paths: &AppPaths) -> Result<(), ()> {
    // All names have been checked as ordinary files. Expiry is enabled only
    // after explicit saved retention choice; no default or reader rotates.
    let oldest = paths.data_file(&format!("audit.{}", RETENTION_FILES - 1));
    match fs::remove_file(oldest) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(()),
    }
    for index in (1..RETENTION_FILES - 1).rev() {
        let from = paths.data_file(&format!("audit.{index}"));
        if from.exists() {
            fs::rename(from, paths.data_file(&format!("audit.{}", index + 1))).map_err(|_| ())?;
        }
    }
    let active = log_path(paths);
    if active.exists() {
        fs::rename(active, paths.data_file("audit.1")).map_err(|_| ())?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RetentionError<E> {
    HistoryUnavailable,
    PreserveRequired,
    Save(E),
}

/// Save the explicit choice while holding the same lock used by the writer.
/// The callback must be synchronous and must not append an audit record.
/// Failed persistence cannot temporarily enable rotation or expiry.
pub(crate) fn transact_retention<T, E>(
    paths: &AppPaths,
    selected: &AtomicBool,
    mode: RetentionMode,
    save: impl FnOnce() -> Result<T, E>,
) -> Result<T, RetentionError<E>> {
    let _guard = WRITE_LOCK.lock();
    if mode == RetentionMode::Bounded {
        let sizes = history_sizes(paths).map_err(|_| RetentionError::HistoryUnavailable)?;
        if sizes.iter().any(|(_, size)| *size > RETENTION_FILE_BYTES) {
            return Err(RetentionError::PreserveRequired);
        }
    }
    let result = save().map_err(RetentionError::Save)?;
    selected.store(mode == RetentionMode::Bounded, Ordering::SeqCst);
    Ok(result)
}

pub(crate) fn set_retention(
    paths: &AppPaths,
    selected: &AtomicBool,
    mode: RetentionMode,
) -> Result<(), ()> {
    transact_retention(paths, selected, mode, || {
        Ok::<_, std::convert::Infallible>(())
    })
    .map_err(|_| ())
}

pub(crate) fn history_status(paths: &AppPaths) -> Result<Value, ()> {
    let _guard = WRITE_LOCK.lock();
    let sizes = history_sizes(paths)?;
    let active = sizes
        .iter()
        .find(|(name, _)| name == "audit.log")
        .map_or(0, |(_, size)| *size);
    let total = sizes
        .iter()
        .fold(0u64, |total, (_, size)| total.saturating_add(*size));
    let oversized = sizes.iter().any(|(_, size)| *size > RETENTION_FILE_BYTES);
    Ok(
        json!({"ok":true,"location":log_path(paths).to_string_lossy(),"active_bytes":active,"total_bytes":total,
        "known_files":sizes.len(),"oversized_legacy":oversized,"preserve_required":oversized,
        "limits":{"files":RETENTION_FILES,"bytes_per_file":RETENTION_FILE_BYTES,"total_bytes":RETENTION_FILES as u64*RETENTION_FILE_BYTES},
        "expiry":"When bounded retention is enabled, the oldest of five files expires before an append rotates the active file. Preserved history is never expired by this policy."}),
    )
}

/// Explicit preservation moves only the five known history names into a fresh
/// exclusive directory. It never overwrites or deletes an existing archive.
pub(crate) fn preserve_history(paths: &AppPaths) -> Result<Value, ()> {
    let _guard = WRITE_LOCK.lock();
    let sizes = history_sizes(paths)?;
    if sizes.is_empty() {
        return Ok(json!({"ok":true,"preserved_files":0,"preserved_bytes":0}));
    }
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut destination = None;
    for _ in 0..16 {
        let count = NEXT_PRESERVATION.fetch_add(1, Ordering::Relaxed);
        let candidate = paths.data_file(&format!("audit-preserved-{epoch:x}-{count:x}"));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                destination = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(()),
        }
    }
    let destination = destination.ok_or(())?;
    let mut moved: Vec<&str> = Vec::new();
    for (name, _) in &sizes {
        if fs::rename(paths.data_file(name), destination.join(name)).is_err() {
            // Best-effort rollback never overwrites a newly created file. Any
            // files that cannot be restored remain preserved in this directory.
            for previous in moved.iter().rev() {
                if !paths.data_file(previous).exists() {
                    let _ = fs::rename(destination.join(previous), paths.data_file(previous));
                }
            }
            return Err(());
        }
        moved.push(name.as_str());
    }
    Ok(
        json!({"ok":true,"preserved_files":sizes.len(),"preserved_bytes":sizes.iter().fold(0u64,|total,(_,size)|total.saturating_add(*size)),
        "preserved_location":destination.to_string_lossy()}),
    )
}

pub(crate) use crate::audit_reader::AuditPage;
pub(crate) fn read_page(
    paths: &AppPaths,
    cursor: Option<&str>,
    limit: usize,
) -> Result<AuditPage, ()> {
    crate::audit_reader::read_page(paths, cursor, limit.min(MAX_TAIL_ENTRIES))
}
#[cfg(test)]
pub fn try_tail(paths: &AppPaths, limit: usize) -> Result<Vec<Value>, ()> {
    Ok(read_page(paths, None, limit.min(MAX_TAIL_ENTRIES))?.entries)
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
    #[test]
    fn oversized_legacy_history_stays_preserved_until_explicit_move_and_opt_in() {
        let dir = TestDir::new();
        let paths = dir.paths();
        fs::create_dir_all(log_path(&paths).parent().unwrap()).unwrap();
        let file = fs::File::create(log_path(&paths)).unwrap();
        file.set_len(100 * 1024 * 1024).unwrap();
        drop(file);
        let retention = AtomicBool::new(false);
        assert!(history_status(&paths).unwrap()["preserve_required"]
            .as_bool()
            .unwrap());
        assert!(set_retention(&paths, &retention, RetentionMode::Bounded).is_err());
        assert!(!retention.load(Ordering::SeqCst));
        append(&paths, json!({"event":"synthetic-later"}), 1.0).unwrap();
        assert!(fs::metadata(log_path(&paths)).unwrap().len() > 100 * 1024 * 1024);
        let preserved = preserve_history(&paths).unwrap();
        assert_eq!(preserved["preserved_files"], 1);
        let archived = std::path::PathBuf::from(preserved["preserved_location"].as_str().unwrap())
            .join("audit.log");
        assert!(archived.exists());
        assert!(!log_path(&paths).exists());
        set_retention(&paths, &retention, RetentionMode::Bounded).unwrap();
        append_record(
            &paths,
            &encoded(&json!({"event":"new-history"}), 2.0).unwrap(),
            &retention,
        )
        .unwrap();
        assert_eq!(try_tail(&paths, 10).unwrap()[0]["event"], "new-history");
        assert!(fs::metadata(archived).unwrap().len() > 100 * 1024 * 1024);
    }

    #[test]
    fn failed_settings_save_cannot_activate_rotation() {
        let dir = TestDir::new();
        let paths = dir.paths();
        fs::create_dir_all(log_path(&paths).parent().unwrap()).unwrap();
        fs::File::create(log_path(&paths))
            .unwrap()
            .set_len(RETENTION_FILE_BYTES)
            .unwrap();
        fs::write(paths.data_file("audit.4"), "synthetic-oldest").unwrap();
        let retention = AtomicBool::new(false);
        let failed = transact_retention(&paths, &retention, RetentionMode::Bounded, || {
            assert!(!retention.load(Ordering::SeqCst));
            Err::<(), _>("synthetic-save-failure")
        });
        assert_eq!(failed, Err(RetentionError::Save("synthetic-save-failure")));
        assert!(!retention.load(Ordering::SeqCst));
        append_record(
            &paths,
            &encoded(&json!({"event":"after-failed-settings-save"}), 1.0).unwrap(),
            &retention,
        )
        .unwrap();
        assert!(fs::metadata(log_path(&paths)).unwrap().len() > RETENTION_FILE_BYTES);
        assert_eq!(
            fs::read_to_string(paths.data_file("audit.4")).unwrap(),
            "synthetic-oldest"
        );
        assert!(!paths.data_file("audit.1").exists());
    }

    #[test]
    fn retention_commit_checks_eligibility_before_save_and_activates_after_success() {
        let dir = TestDir::new();
        let paths = dir.paths();
        fs::create_dir_all(log_path(&paths).parent().unwrap()).unwrap();
        fs::File::create(log_path(&paths))
            .unwrap()
            .set_len(RETENTION_FILE_BYTES + 1)
            .unwrap();
        let retention = AtomicBool::new(false);
        let failure = transact_retention(
            &paths,
            &retention,
            RetentionMode::Bounded,
            || -> Result<(), ()> {
                panic!("an ineligible retention choice must not reach settings save");
            },
        );
        assert_eq!(failure, Err(RetentionError::PreserveRequired));
        preserve_history(&paths).unwrap();
        let saved = transact_retention(&paths, &retention, RetentionMode::Bounded, || {
            assert!(!retention.load(Ordering::SeqCst));
            Ok::<_, ()>("synthetic-saved")
        })
        .unwrap();
        assert_eq!(saved, "synthetic-saved");
        assert!(retention.load(Ordering::SeqCst));
    }

    #[test]
    fn opted_in_rotation_expires_only_the_oldest_known_file_before_append() {
        let dir = TestDir::new();
        let paths = dir.paths();
        fs::create_dir_all(log_path(&paths).parent().unwrap()).unwrap();
        let active = fs::File::create(log_path(&paths)).unwrap();
        active.set_len(RETENTION_FILE_BYTES).unwrap();
        drop(active);
        for index in 1..RETENTION_FILES {
            fs::write(
                paths.data_file(&format!("audit.{index}")),
                format!("synthetic-{index}"),
            )
            .unwrap();
        }
        fs::write(paths.data_file("unrelated.keep"), "synthetic-unrelated").unwrap();
        let retention = AtomicBool::new(false);
        set_retention(&paths, &retention, RetentionMode::Bounded).unwrap();
        append_record(
            &paths,
            &encoded(&json!({"event":"after-rotation"}), 1.0).unwrap(),
            &retention,
        )
        .unwrap();
        assert_eq!(
            fs::metadata(paths.data_file("audit.1")).unwrap().len(),
            RETENTION_FILE_BYTES
        );
        assert_eq!(
            fs::read_to_string(paths.data_file("audit.4")).unwrap(),
            "synthetic-3"
        );
        assert_eq!(
            fs::read_to_string(paths.data_file("unrelated.keep")).unwrap(),
            "synthetic-unrelated"
        );
        assert_eq!(try_tail(&paths, 10).unwrap()[0]["event"], "after-rotation");
        assert!(
            history_status(&paths).unwrap()["total_bytes"]
                .as_u64()
                .unwrap()
                <= RETENTION_FILES as u64 * RETENTION_FILE_BYTES
        );
    }

    #[test]
    fn repeated_preservation_creates_distinct_archives_and_preserve_mode_never_prunes() {
        let dir = TestDir::new();
        let paths = dir.paths();
        append(&paths, json!({"event":"first"}), 1.0).unwrap();
        let first = preserve_history(&paths).unwrap();
        append(&paths, json!({"event":"second"}), 2.0).unwrap();
        let second = preserve_history(&paths).unwrap();
        assert_ne!(first["preserved_location"], second["preserved_location"]);
        for preserved in [first, second] {
            assert!(
                std::path::Path::new(preserved["preserved_location"].as_str().unwrap())
                    .join("audit.log")
                    .exists()
            );
        }
        assert_eq!(preserve_history(&paths).unwrap()["preserved_files"], 0);
    }

    #[cfg(unix)]
    #[test]
    fn history_symlinks_are_not_followed_by_retention_or_preservation() {
        let dir = TestDir::new();
        let paths = dir.paths();
        fs::create_dir_all(log_path(&paths).parent().unwrap()).unwrap();
        let target = dir.path().join("synthetic-external-log");
        fs::write(&target, "synthetic-original").unwrap();
        std::os::unix::fs::symlink(&target, log_path(&paths)).unwrap();
        assert!(history_status(&paths).is_err());
        assert!(preserve_history(&paths).is_err());
        assert!(append(&paths, json!({"event":"blocked"}), 1.0).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "synthetic-original");
    }
    #[cfg(unix)]
    #[test]
    fn append_and_flush_reject_symlinks_without_modifying_the_target() {
        let dir = TestDir::new();
        let paths = dir.paths();
        let path = log_path(&paths);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = dir.path().join("synthetic-unrelated-target");
        fs::write(&target, b"synthetic-target-must-stay-unchanged\n").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let before = fs::read(&target).unwrap();
        assert!(append(&paths, json!({"event":"synthetic-attempt"}), 1.0).is_err());
        assert!(sync_history(&paths).is_err());
        assert_eq!(fs::read(&target).unwrap(), before);
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn raced_in_symlinks_cannot_open_for_append_flush_or_new_history() {
        let dir = TestDir::new();
        let paths = dir.paths();
        let path = log_path(&paths);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"synthetic-original-history\n").unwrap();
        let inspected = fs::symlink_metadata(&path).unwrap();
        let moved = dir.path().join("synthetic-original-handle-target");
        fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &path).unwrap();
        for append in [false, true] {
            let mut options = no_follow_options();
            if append {
                options.append(true);
            } else {
                options.write(true);
            }
            // Same inode and metadata as the inspected original; no-follow and
            // current symlink metadata must reject it before content operations.
            assert!(checked_open(&path, Some(&inspected), &options).is_err());
        }
        let mut create = no_follow_options();
        create.create_new(true).append(true);
        assert!(checked_open(&path, None, &create).is_err());
        assert_eq!(fs::read(&moved).unwrap(), b"synthetic-original-history\n");
    }
}
