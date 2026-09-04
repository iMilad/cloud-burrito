//! Bounded audit tail snapshots and append cursors. Cursors contain fingerprints,
//! never paths or log contents, and are sampled continuity hints rather than authority.
//! A truncate/regrow or interior rewrite combined with append can evade the head
//! and boundary samples; this is not universal file-edit detection.

use crate::audit::{file_identity as identity, regular_file as regular};
use crate::paths::AppPaths;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, File, Metadata},
    hash::{Hash, Hasher},
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::UNIX_EPOCH,
};

const MAX_READ_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_RECORD_BYTES: usize = 16 * 1024;
const MAX_ENTRIES: usize = 1000;
const BLOCK: usize = 64 * 1024;
const ANCHOR: usize = 64;
const READ_RESERVE: usize = 512;
const RESPONSE_RESERVE: usize = 4096;

#[derive(Debug, Serialize)]
pub(crate) struct AuditPage {
    pub entries: Vec<Value>,
    pub cursor: Option<String>,
    pub bytes_read: usize,
    pub skipped: usize,
    /// More forward backlog remains. The initial tail does not page backwards.
    pub has_more: bool,
    /// A supplied cursor could not be continued; replace the displayed page.
    pub reset: bool,
    pub limited: bool,
    /// An unfinished final record is withheld until its terminating newline.
    pub partial_tail: bool,
}
impl AuditPage {
    fn empty(reset: bool) -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            bytes_read: 0,
            skipped: 0,
            has_more: false,
            reset,
            limited: false,
            partial_tail: false,
        }
    }
}
#[derive(Clone, Copy)]
struct Cursor {
    identity: u64,
    offset: u64,
    length: u64,
    modified: u128,
    head: u64,
    anchor: u64,
    discard: bool,
}
impl Cursor {
    fn parse(text: &str) -> Option<Self> {
        if text.len() > 256 || !text.is_ascii() {
            return None;
        }
        let parts: Vec<_> = text.split(':').collect();
        if parts.len() != 8 || parts[0] != "a1" {
            return None;
        }
        if parts[1..7]
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return None;
        }
        let hex = |i: usize| u64::from_str_radix(parts[i], 16).ok();
        let offset = hex(2)?;
        let length = hex(3)?;
        if offset > length {
            return None;
        }
        Some(Self {
            identity: hex(1)?,
            offset,
            length,
            modified: u128::from_str_radix(parts[4], 16).ok()?,
            head: hex(5)?,
            anchor: hex(6)?,
            discard: match parts[7] {
                "0" => false,
                "1" => true,
                _ => return None,
            },
        })
    }
    fn encode(self) -> String {
        format!(
            "a1:{:016x}:{:x}:{:x}:{:x}:{:016x}:{:016x}:{}",
            self.identity,
            self.offset,
            self.length,
            self.modified,
            self.head,
            self.anchor,
            u8::from(self.discard)
        )
    }
}

fn modified(metadata: &Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos())
}
fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    hash.finish()
}

fn open_regular(path: &Path, inspected: &Metadata) -> Result<(File, Metadata), ()> {
    let mut options = crate::audit::no_follow_options();
    options.read(true);
    crate::audit::checked_open(path, Some(inspected), &options)
}

fn snapshot_still_current(path: &Path, file_id: u64, length: u64, modified_at: u128) -> bool {
    let Ok(current) = fs::symlink_metadata(path) else {
        return false;
    };
    regular(&current)
        && identity(&current) == file_id
        && current.len() >= length
        && (current.len() != length || modified(&current) == modified_at)
}

struct Meter {
    file: File,
    bytes: usize,
}
impl Meter {
    fn read_at(&mut self, start: u64, length: usize) -> Result<Vec<u8>, ()> {
        if length > MAX_READ_BYTES.saturating_sub(self.bytes) {
            return Err(());
        }
        self.file.seek(SeekFrom::Start(start)).map_err(|_| ())?;
        let mut bytes = vec![0; length];
        let mut done = 0;
        while done < length {
            let read = self.file.read(&mut bytes[done..]).map_err(|_| ())?;
            self.bytes += read;
            if read == 0 {
                return Err(());
            }
            done += read;
        }
        Ok(bytes)
    }
    fn head(&mut self, length: u64) -> Result<u64, ()> {
        Ok(fingerprint(
            &self.read_at(0, length.min(ANCHOR as u64) as usize)?,
        ))
    }
    fn anchor(&mut self, offset: u64) -> Result<u64, ()> {
        let length = offset.min(ANCHOR as u64) as usize;
        Ok(fingerprint(&self.read_at(offset - length as u64, length)?))
    }
    fn available(&self) -> usize {
        MAX_READ_BYTES.saturating_sub(READ_RESERVE + self.bytes)
    }
}

struct Rows {
    entries: Vec<Value>,
    bytes: usize,
    skipped: usize,
}
impl Rows {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            bytes: 0,
            skipped: 0,
        }
    }
    /// False means this complete valid record must be retried on the next page.
    fn retain(&mut self, line: &[u8]) -> bool {
        if line.is_empty() || line.len() > MAX_RECORD_BYTES {
            self.skipped += 1;
            return true;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            self.skipped += 1;
            return true;
        };
        if !value.is_object() {
            self.skipped += 1;
            return true;
        }
        let mut safe = crate::audit::sanitized(&value);
        if let Some(ts) = value.get("ts").and_then(Value::as_f64) {
            safe.insert("ts".into(), serde_json::json!(ts));
        }
        let value = Value::Object(safe);
        // Sanitization retains only bounded identifiers and fixed diagnostic fields.
        let bytes = serde_json::to_vec(&value).map_or(MAX_RESPONSE_BYTES, |value| value.len() + 1);
        if bytes > MAX_RESPONSE_BYTES.saturating_sub(RESPONSE_RESERVE + self.bytes) {
            return false;
        }
        self.bytes += bytes;
        self.entries.push(value);
        true
    }
}
struct Progress {
    offset: u64,
    discard: bool,
    has_more: bool,
    limited: bool,
    partial: bool,
}

fn reverse(meter: &mut Meter, length: u64, limit: usize, rows: &mut Rows) -> Result<Progress, ()> {
    let mut position = length;
    let mut line = Vec::new();
    let mut oversized = false;
    let mut counted = false;
    let mut final_segment = true;
    let mut partial = false;
    let mut offset = length;
    let mut discard = false;
    let mut limited = false;
    let mut stopped = false;
    while position > 0 && meter.available() > 0 && !stopped {
        let size = (position.min(BLOCK as u64) as usize).min(meter.available());
        let start = position - size as u64;
        let chunk = meter.read_at(start, size)?;
        if position == length {
            partial = chunk.last() != Some(&b'\n');
        }
        for (index, byte) in chunk.iter().enumerate().rev() {
            let absolute = start + index as u64;
            if *byte == b'\n' {
                if final_segment {
                    // The first segment is either empty after the terminal
                    // newline or unfinished data that must not be emitted yet.
                    offset = if partial && !oversized {
                        absolute + 1
                    } else {
                        length
                    };
                    discard = partial && oversized;
                    final_segment = false;
                } else if oversized {
                    if !counted {
                        rows.skipped += 1;
                    }
                } else {
                    line.reverse();
                    if !rows.retain(&line) {
                        limited = true;
                        stopped = true;
                        break;
                    }
                }
                line.clear();
                oversized = false;
                counted = false;
                if rows.entries.len() == limit {
                    limited = true;
                    stopped = true;
                    break;
                }
            } else if !oversized {
                if line.len() == MAX_RECORD_BYTES {
                    oversized = true;
                    line.clear();
                    if !final_segment {
                        rows.skipped += 1;
                        counted = true;
                    }
                } else {
                    line.push(*byte);
                }
            }
        }
        position = start;
    }
    if !stopped {
        if position == 0 {
            if final_segment {
                offset = if oversized { length } else { 0 };
                discard = oversized;
            } else if oversized {
                if !counted {
                    rows.skipped += 1;
                }
            } else {
                line.reverse();
                if !rows.retain(&line) {
                    limited = true;
                }
            }
        } else {
            limited = true;
            if final_segment {
                offset = length;
                discard = true;
            }
        }
    }
    rows.entries.reverse();
    Ok(Progress {
        offset,
        discard,
        has_more: false,
        limited,
        partial,
    })
}

fn forward(
    meter: &mut Meter,
    cursor: Cursor,
    length: u64,
    limit: usize,
    rows: &mut Rows,
) -> Result<Progress, ()> {
    let mut position = cursor.offset;
    let mut record_start = position;
    let mut offset = position;
    let mut discard = cursor.discard;
    let mut line = Vec::new();
    let mut stop = false;
    let mut limited = false;
    let mut partial = false;
    while position < length && meter.available() > 0 && !stop {
        let size = ((length - position).min(BLOCK as u64) as usize).min(meter.available());
        let chunk = meter.read_at(position, size)?;
        for (index, byte) in chunk.iter().enumerate() {
            let next = position + index as u64 + 1;
            if *byte == b'\n' {
                if discard {
                    rows.skipped += 1;
                    discard = false;
                } else if !rows.retain(&line) {
                    offset = record_start;
                    limited = true;
                    stop = true;
                    break;
                }
                line.clear();
                record_start = next;
                offset = next;
                if rows.entries.len() == limit {
                    stop = true;
                    limited = next < length;
                    break;
                }
            } else if !discard {
                if line.len() == MAX_RECORD_BYTES {
                    discard = true;
                    line.clear();
                } else {
                    line.push(*byte);
                }
            }
            // Oversized records can be skipped over multiple bounded calls.
            if discard {
                offset = next;
            }
        }
        position += size as u64;
    }
    if !stop {
        let at_end = position == length;
        partial = at_end && (discard || !line.is_empty());
        if !discard {
            offset = record_start;
        }
        limited = !at_end;
    }
    let has_more = limited && offset < length;
    Ok(Progress {
        offset,
        discard,
        has_more,
        limited,
        partial,
    })
}

pub(crate) fn read_page(
    paths: &AppPaths,
    cursor: Option<&str>,
    limit: usize,
) -> Result<AuditPage, ()> {
    if limit == 0 {
        return Ok(AuditPage::empty(false));
    }
    let path = crate::audit::log_path(paths);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AuditPage::empty(cursor.is_some()))
        }
        Err(_) => return Err(()),
    };
    if !regular(&metadata) {
        return Err(());
    }
    let (file, metadata) = open_regular(&path, &metadata)?;
    let length = metadata.len();
    let file_id = identity(&metadata);
    let modified_at = modified(&metadata);
    let mut meter = Meter { file, bytes: 0 };
    let mut reset = false;
    let parsed = cursor.and_then(Cursor::parse);
    let continuation = if let Some(previous) = parsed {
        let valid = previous.identity == file_id
            && previous.offset <= length
            && previous.length <= length
            && (previous.length != length || previous.modified == modified_at)
            && meter.head(previous.length)? == previous.head
            && meter.anchor(previous.offset)? == previous.anchor;
        if valid {
            Some(previous)
        } else {
            reset = true;
            None
        }
    } else {
        reset = cursor.is_some();
        None
    };
    let mut rows = Rows::new();
    let progress = if let Some(previous) = continuation {
        forward(
            &mut meter,
            previous,
            length,
            limit.min(MAX_ENTRIES),
            &mut rows,
        )?
    } else {
        reverse(&mut meter, length, limit.min(MAX_ENTRIES), &mut rows)?
    };
    let head = meter.head(length)?;
    let anchor = meter.anchor(progress.offset)?;
    // Appends after the snapshot are left for the next cursor. A replacement
    // or same-length edit during this read is retried instead of mixing files.
    if !snapshot_still_current(&path, file_id, length, modified_at) {
        return Err(());
    }
    let next = Cursor {
        identity: file_id,
        offset: progress.offset,
        length,
        modified: modified_at,
        head,
        anchor,
        discard: progress.discard,
    }
    .encode();
    Ok(AuditPage {
        entries: rows.entries,
        cursor: Some(next),
        bytes_read: meter.bytes,
        skipped: rows.skipped,
        has_more: progress.has_more,
        reset,
        limited: progress.limited,
        partial_tail: progress.partial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use serde_json::json;
    use std::io::{BufWriter, Write};

    fn path(dir: &TestDir) -> std::path::PathBuf {
        let path = dir.paths().data_file("audit.log");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        path
    }
    fn line(ts: usize) -> String {
        format!("{{\"kind\":\"request\",\"event\":\"succeeded\",\"ts\":{ts}}}\n")
    }
    fn append(path: &std::path::Path, text: &[u8]) {
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text)
            .unwrap();
    }
    fn assert_bounds(page: &AuditPage) {
        assert!(page.bytes_read <= MAX_READ_BYTES);
        assert!(page.entries.len() <= MAX_ENTRIES);
        assert!(serde_json::to_vec(page).unwrap().len() <= MAX_RESPONSE_BYTES);
        if let Some(cursor) = &page.cursor {
            assert!(cursor.len() <= 256 && cursor.is_ascii());
        }
    }

    #[test]
    fn last_three_hundred_entries_do_not_scan_one_ten_or_one_hundred_mib_histories() {
        for mib in [1usize, 10, 100] {
            let dir = TestDir::new();
            let path = path(&dir);
            let mut file = BufWriter::new(File::create(&path).unwrap());
            let repeated = line(0);
            let block = repeated.repeat(BLOCK / repeated.len());
            let mut written = 0;
            while written < mib * 1024 * 1024 {
                file.write_all(block.as_bytes()).unwrap();
                written += block.len();
            }
            for index in 1..=400 {
                file.write_all(line(index).as_bytes()).unwrap();
            }
            file.flush().unwrap();
            drop(file);
            let page = read_page(&dir.paths(), None, 300).unwrap();
            assert_bounds(&page);
            assert!(page.bytes_read < 256 * 1024);
            assert_eq!(page.entries.len(), 300);
            assert_eq!(page.entries.first().unwrap()["ts"], 101.0);
            assert_eq!(page.entries.last().unwrap()["ts"], 400.0);
            assert!(page.limited);
            assert!(!page.has_more);
        }
    }

    #[test]
    fn partial_final_record_is_ignored_then_delivered_once_after_newline() {
        let dir = TestDir::new();
        let path = path(&dir);
        fs::write(
            &path,
            format!("{}{}{{\"kind\":\"request\",\"ts\":3", line(1), line(2)),
        )
        .unwrap();
        let first = read_page(&dir.paths(), None, 300).unwrap();
        assert_eq!(first.entries.len(), 2);
        assert!(first.partial_tail);
        let empty = read_page(&dir.paths(), first.cursor.as_deref(), 300).unwrap();
        assert!(empty.entries.is_empty());
        assert!(empty.partial_tail);
        assert!(!empty.has_more);
        append(&path, format!("}}\n{}", line(4)).as_bytes());
        let next = read_page(&dir.paths(), empty.cursor.as_deref(), 300).unwrap();
        assert_eq!(
            next.entries
                .iter()
                .map(|row| row["ts"].as_f64().unwrap())
                .collect::<Vec<_>>(),
            vec![3.0, 4.0]
        );
        assert!(!next.reset);
        assert!(!next.partial_tail);
        assert_bounds(&next);
        assert!(read_page(&dir.paths(), next.cursor.as_deref(), 300)
            .unwrap()
            .entries
            .is_empty());
    }

    #[test]
    fn forward_backlog_paginates_without_duplicates_or_discarding_a_budget_blocked_row() {
        let dir = TestDir::new();
        let path = path(&dir);
        fs::write(&path, line(0)).unwrap();
        let mut page = read_page(&dir.paths(), None, 300).unwrap();
        append(&path, (1..=10).map(line).collect::<String>().as_bytes());
        let mut seen = Vec::new();
        loop {
            page = read_page(&dir.paths(), page.cursor.as_deref(), 3).unwrap();
            assert_bounds(&page);
            seen.extend(
                page.entries
                    .iter()
                    .map(|row| row["ts"].as_f64().unwrap() as usize),
            );
            if !page.has_more {
                break;
            }
        }
        assert_eq!(seen, (1..=10).collect::<Vec<_>>());

        let mut record = serde_json::Map::new();
        for key in [
            "kind",
            "event",
            "command",
            "widget",
            "service",
            "operation",
            "error_type",
            "request_id",
            "account_id",
            "region",
        ] {
            record.insert(key.into(), json!("x".repeat(128)));
        }
        let mut appended = String::new();
        for index in 1..=1000 {
            record.insert("ts".into(), json!(index));
            appended.push_str(&format!("{}\n", Value::Object(record.clone())));
        }
        append(&path, appended.as_bytes());
        let mut count = 0;
        loop {
            page = read_page(&dir.paths(), page.cursor.as_deref(), 1000).unwrap();
            assert_bounds(&page);
            count += page.entries.len();
            if !page.has_more {
                break;
            }
        }
        assert_eq!(count, 1000);
    }

    #[test]
    fn malformed_oversized_and_private_fields_are_accounted_for_without_partial_json() {
        let dir = TestDir::new();
        let path = path(&dir);
        fs::write(&path,format!("not-json\n[]\n{{}}\n{}\n{{\"kind\":\"request\",\"ts\":9,\"secret\":\"SYNTHETIC_PRIVATE_AUDIT_PAYLOAD\"}}\nunfinished","x".repeat(MAX_RECORD_BYTES+1))).unwrap();
        let page = read_page(&dir.paths(), None, 300).unwrap();
        assert_bounds(&page);
        assert_eq!(page.skipped, 3);
        assert_eq!(page.entries.len(), 2);
        assert!(page.partial_tail);
        assert_eq!(page.entries[1]["ts"], 9.0);
        assert!(!serde_json::to_string(&page)
            .unwrap()
            .contains("SYNTHETIC_PRIVATE_AUDIT_PAYLOAD"));
    }

    #[test]
    fn a_huge_oversized_forward_record_is_skipped_across_bounded_calls_once() {
        let dir = TestDir::new();
        let path = path(&dir);
        fs::write(&path, []).unwrap();
        let mut page = read_page(&dir.paths(), None, 300).unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        let block = vec![b'x'; BLOCK];
        for _ in 0..(MAX_READ_BYTES * 2 / BLOCK) {
            file.write_all(&block).unwrap();
        }
        file.write_all(b"xxxxxxxxxx\n").unwrap();
        file.write_all(line(1).as_bytes()).unwrap();
        drop(file);
        let mut skipped = 0;
        let mut entries = Vec::new();
        let mut calls = 0;
        loop {
            page = read_page(&dir.paths(), page.cursor.as_deref(), 300).unwrap();
            assert_bounds(&page);
            calls += 1;
            skipped += page.skipped;
            entries.extend(page.entries.clone());
            if !page.has_more {
                break;
            }
            assert!(calls < 5);
        }
        assert_eq!(skipped, 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["ts"], 1.0);
        assert!(calls >= 3);
    }

    #[test]
    fn replacement_truncation_same_length_rewrite_and_invalid_cursors_reset_the_page() {
        let dir = TestDir::new();
        let path = path(&dir);
        fs::write(&path, (1..=1000).map(line).collect::<String>()).unwrap();
        let first = read_page(&dir.paths(), None, 300).unwrap();
        let replacement = dir.path().join("synthetic-replacement");
        fs::write(&replacement, line(8)).unwrap();
        fs::remove_file(&path).unwrap();
        fs::rename(replacement, &path).unwrap();
        let replaced = read_page(&dir.paths(), first.cursor.as_deref(), 300).unwrap();
        assert!(replaced.reset);
        assert_eq!(replaced.entries[0]["ts"], 8.0);
        fs::write(&path, []).unwrap();
        let truncated = read_page(&dir.paths(), replaced.cursor.as_deref(), 300).unwrap();
        assert!(truncated.reset);
        assert!(truncated.entries.is_empty());
        fs::write(&path, (1..=1000).map(line).collect::<String>()).unwrap();
        let first = read_page(&dir.paths(), None, 300).unwrap();
        // Change a middle byte outside both cursor anchors, keeping the length.
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(2000)).unwrap();
        file.write_all(b"x").unwrap();
        file.set_times(
            fs::FileTimes::new()
                .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000)),
        )
        .unwrap();
        drop(file);
        assert!(
            read_page(&dir.paths(), first.cursor.as_deref(), 300)
                .unwrap()
                .reset
        );
        for invalid in ["not-a-cursor".to_string(), "x".repeat(257), "🦀".into()] {
            assert!(read_page(&dir.paths(), Some(&invalid), 300).unwrap().reset);
        }
    }

    #[test]
    fn missing_history_is_normal_and_non_regular_history_is_an_error() {
        let dir = TestDir::new();
        let absent = read_page(&dir.paths(), None, 300).unwrap();
        assert!(absent.entries.is_empty());
        assert_eq!(absent.bytes_read, 0);
        assert!(absent.cursor.is_none());
        assert!(
            read_page(&dir.paths(), Some("old-cursor"), 300)
                .unwrap()
                .reset
        );
        fs::create_dir_all(path(&dir)).unwrap();
        assert!(read_page(&dir.paths(), None, 300).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_audit_symlink_is_not_followed() {
        let dir = TestDir::new();
        let path = path(&dir);
        let target = dir.path().join("synthetic-other-data");
        fs::write(&target, line(1)).unwrap();
        std::os::unix::fs::symlink(target, path).unwrap();
        assert!(read_page(&dir.paths(), None, 300).is_err());
    }
    #[test]
    fn replacement_between_inspection_and_open_is_rejected_before_reading() {
        let dir = TestDir::new();
        let path = path(&dir);
        fs::write(&path, line(1)).unwrap();
        let inspected = fs::symlink_metadata(&path).unwrap();
        // Hold the original file so its identity cannot be recycled after rename.
        let original = File::open(&path).unwrap();
        let moved = dir.path().join("synthetic-original-history");
        fs::rename(&path, &moved).unwrap();
        fs::write(&path, line(2)).unwrap();
        assert!(open_regular(&path, &inspected).is_err());
        assert!(!snapshot_still_current(
            &path,
            identity(&inspected),
            inspected.len(),
            modified(&inspected)
        ));
        drop(original);
        // A fresh read can use the replacement and explicitly resets an old cursor.
        assert_eq!(
            read_page(&dir.paths(), None, 300).unwrap().entries[0]["ts"],
            2.0
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_substitution_before_open_or_after_open_cannot_pass_identity_checks() {
        let dir = TestDir::new();
        let path = path(&dir);
        fs::write(&path, line(1)).unwrap();
        let inspected = fs::symlink_metadata(&path).unwrap();
        let (opened, snapshot) = open_regular(&path, &inspected).unwrap();
        let moved = dir.path().join("synthetic-same-inode-target");
        fs::rename(&path, &moved).unwrap();
        // The target has exactly the same inode, bytes and timestamps; an ordinary
        // metadata(path) comparison would follow it and incorrectly accept it.
        std::os::unix::fs::symlink(&moved, &path).unwrap();
        assert!(open_regular(&path, &inspected).is_err());
        assert!(!snapshot_still_current(
            &path,
            identity(&snapshot),
            snapshot.len(),
            modified(&snapshot)
        ));
        assert!(read_page(&dir.paths(), None, 300).is_err());
        drop(opened);
    }
}
