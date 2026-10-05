//! Bounded JSON storage with ordered, atomic replacement.
//!
//! The shared Store serializes replacement per file. It does not certify
//! power-loss durability, cross-process ordering, or native platform behavior.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::paths::AppPaths;

// Allows pretty-printed legacy files while IPC retains its tighter JSON budget.
pub(crate) const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StorageError {
    ReadFailed,
    Invalid,
    WriteFailed,
    Superseded,
}

impl StorageError {
    pub(crate) fn response(self, store: &str) -> Value {
        let (error_type, message) = match self {
            Self::ReadFailed => (
                "StorageReadFailed",
                "Saved application data could not be read. Check local file access and retry.",
            ),
            Self::Invalid => (
                "StorageInvalid",
                "Application storage data is malformed or exceeds supported limits. Review or restore it before saving.",
            ),
            Self::WriteFailed => (
                "StorageWriteFailed",
                "Changes could not be saved; the previous saved file was kept.",
            ),
            Self::Superseded => (
                "StorageSuperseded",
                "A newer save superseded these changes.",
            ),
        };
        // Callers use a fixed store name, never a filesystem path.
        let store = match store {
            "settings" => "settings",
            "dashboard" => "dashboard",
            _ => "application",
        };
        json!({"ok":false, "error_type":error_type, "error":message,
            "_storage":{"store":store, "status":"failed"}})
    }
}

pub(crate) trait WritableFile: Write + Send {
    fn sync_all(&mut self) -> io::Result<()>;
}

impl WritableFile for File {
    fn sync_all(&mut self) -> io::Result<()> {
        File::sync_all(self)
    }
}

/// Injectable operations; test adapters only receive explicit disposable paths.
pub(crate) trait FileSystem: Send + Sync {
    fn read_bounded(&self, path: &Path, max: usize) -> io::Result<Option<Vec<u8>>>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn create_new(&self, path: &Path) -> io::Result<Box<dyn WritableFile>>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
}

pub(crate) struct NativeFileSystem;

impl FileSystem for NativeFileSystem {
    fn read_bounded(&self, path: &Path, max: usize) -> io::Result<Option<Vec<u8>>> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(max as u64 + 1).read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        crate::file_privacy::ensure_directory(path)
    }

    fn create_new(&self, path: &Path) -> io::Result<Box<dyn WritableFile>> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(path)
            .map(|file| Box::new(file) as Box<dyn WritableFile>)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        // No remove-destination fallback: a failure must keep the old file.
        fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}

#[derive(Default)]
struct FileOrder {
    next_ticket: AtomicU64,
    replacement: Mutex<()>,
}

#[derive(Clone)]
pub(crate) struct Store {
    paths: AppPaths,
    filesystem: Arc<dyn FileSystem>,
    order: Arc<Mutex<HashMap<String, Arc<FileOrder>>>>,
}

impl Store {
    pub(crate) fn new(paths: AppPaths) -> Self {
        Self::with_filesystem(paths, Arc::new(NativeFileSystem))
    }

    pub(crate) fn with_filesystem(paths: AppPaths, filesystem: Arc<dyn FileSystem>) -> Self {
        Self {
            paths,
            filesystem,
            order: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn file_order(&self, name: &str) -> Arc<FileOrder> {
        self.order
            .lock()
            .entry(name.to_owned())
            .or_default()
            .clone()
    }

    pub(crate) fn read_json(&self, name: &str) -> Result<Option<Value>, StorageError> {
        let order = self.file_order(name);
        let _replacement = order.replacement.lock();
        let bytes = self
            .filesystem
            .read_bounded(&self.paths.data_file(name), MAX_FILE_BYTES)
            .map_err(|_| StorageError::ReadFailed)?;
        let Some(bytes) = bytes else { return Ok(None) };
        if bytes.len() > MAX_FILE_BYTES {
            return Err(StorageError::Invalid);
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| StorageError::Invalid)
    }

    pub(crate) fn write_json<T: Serialize + ?Sized>(
        &self,
        name: &str,
        value: &T,
    ) -> Result<(), StorageError> {
        let order = self.file_order(name);
        let ticket = order.next_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        // Serialization can finish out of order. Its ticket still prevents a
        // slow older request from replacing a later accepted request's file.
        let mut output = BoundedOutput(Vec::new());
        serde_json::to_writer_pretty(&mut output, value).map_err(|_| StorageError::WriteFailed)?;
        let _replacement = order.replacement.lock();
        if ticket != order.next_ticket.load(Ordering::SeqCst) {
            return Err(StorageError::Superseded);
        }
        replace_bytes(
            &self.filesystem,
            &self.paths.data_file(name),
            &output.0,
            || ticket == order.next_ticket.load(Ordering::SeqCst),
        )
    }
}

/// Replace a complete file without exposing partial bytes at its live path.
/// The caller owns ordering and validation; `can_replace` is checked after
/// writing and syncing, immediately before the atomic rename.
pub(crate) fn replace_bytes(
    filesystem: &Arc<dyn FileSystem>,
    destination: &Path,
    bytes: &[u8],
    can_replace: impl FnOnce() -> bool,
) -> Result<(), StorageError> {
    let parent = destination.parent().ok_or(StorageError::WriteFailed)?;
    let name = destination.file_name().ok_or(StorageError::WriteFailed)?;
    filesystem
        .create_dir_all(parent)
        .map_err(|_| StorageError::WriteFailed)?;
    let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.{sequence}.tmp", std::process::id()));
    let temporary = parent.join(temporary_name);
    // Cleanup ownership starts only after create_new succeeds. A collision
    // cannot remove or truncate an unrelated existing file.
    let mut file = filesystem
        .create_new(&temporary)
        .map_err(|_| StorageError::WriteFailed)?;
    let mut cleanup = Temporary {
        path: temporary,
        filesystem: filesystem.clone(),
        owned: true,
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    written.map_err(|_| StorageError::WriteFailed)?;
    if !can_replace() {
        return Err(StorageError::Superseded);
    }
    filesystem
        .rename(&cleanup.path, destination)
        .map_err(|_| StorageError::WriteFailed)?;
    cleanup.owned = false;
    Ok(())
}

struct Temporary {
    path: PathBuf,
    filesystem: Arc<dyn FileSystem>,
    owned: bool,
}

impl Drop for Temporary {
    fn drop(&mut self) {
        if self.owned {
            // A failed cleanup may leave this app-owned temp file. The live
            // file remains untouched, and save still reports its original error.
            let _ = self.filesystem.remove_file(&self.path);
        }
    }
}

struct BoundedOutput(Vec<u8>);

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FILE_BYTES - self.0.len() {
            return Err(io::Error::other("storage output exceeds its size limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn status(mut value: Value, store: &str, status: &str) -> Value {
    value["_storage"] = json!({"store":store,"status":status});
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use std::sync::mpsc;
    use std::sync::Barrier;
    use std::time::Duration;

    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        Read,
        Directory,
        Create,
        Write,
        Sync,
        Rename,
    }

    struct FaultFs {
        fault: Fault,
    }

    impl FileSystem for FaultFs {
        fn read_bounded(&self, path: &Path, max: usize) -> io::Result<Option<Vec<u8>>> {
            if self.fault == Fault::Read {
                return Err(io::Error::other("SYNTHETIC_PRIVATE_READ_DETAIL"));
            }
            NativeFileSystem.read_bounded(path, max)
        }
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            if self.fault == Fault::Directory {
                return Err(io::Error::other("synthetic directory failure"));
            }
            NativeFileSystem.create_dir_all(path)
        }
        fn create_new(&self, path: &Path) -> io::Result<Box<dyn WritableFile>> {
            if self.fault == Fault::Create {
                return Err(io::Error::other("synthetic create failure"));
            }
            Ok(Box::new(FaultWriter {
                inner: NativeFileSystem.create_new(path)?,
                fault: self.fault,
                wrote: false,
            }))
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            if self.fault == Fault::Rename {
                return Err(io::Error::other("synthetic rename failure"));
            }
            NativeFileSystem.rename(from, to)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            NativeFileSystem.remove_file(path)
        }
    }

    struct FaultWriter {
        inner: Box<dyn WritableFile>,
        fault: Fault,
        wrote: bool,
    }

    impl Write for FaultWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fault == Fault::Write {
                if self.wrote {
                    return Err(io::Error::other("synthetic partial write failure"));
                }
                self.wrote = true;
                return self.inner.write(&bytes[..bytes.len().min(3)]);
            }
            self.inner.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    impl WritableFile for FaultWriter {
        fn sync_all(&mut self) -> io::Result<()> {
            if self.fault == Fault::Sync {
                return Err(io::Error::other("synthetic sync failure"));
            }
            self.inner.sync_all()
        }
    }

    #[test]
    fn read_errors_corruption_and_oversize_are_distinct_from_missing() {
        let directory = TestDir::new();
        let paths = directory.paths();
        let store = Store::new(paths.clone());
        assert_eq!(store.read_json("fixture.json"), Ok(None));
        assert!(!paths.data_file("fixture.json").exists());
        store
            .write_json("fixture.json", &json!({"saved":"synthetic"}))
            .unwrap();
        let error_store =
            Store::with_filesystem(paths.clone(), Arc::new(FaultFs { fault: Fault::Read }));
        let error = error_store.read_json("fixture.json").unwrap_err();
        assert_eq!(error, StorageError::ReadFailed);
        assert!(!error
            .response("settings")
            .to_string()
            .contains("SYNTHETIC_PRIVATE_READ_DETAIL"));
        for bytes in [
            b"{SYNTHETIC_PRIVATE_INVALID_CONTENT".to_vec(),
            vec![b' '; MAX_FILE_BYTES + 1],
        ] {
            fs::write(paths.data_file("fixture.json"), &bytes).unwrap();
            assert_eq!(store.read_json("fixture.json"), Err(StorageError::Invalid));
            assert_eq!(fs::read(paths.data_file("fixture.json")).unwrap(), bytes);
        }
    }

    #[test]
    fn injected_filesystem_failures_keep_previous_file_and_remove_owned_temporary() {
        for fault in [
            Fault::Directory,
            Fault::Create,
            Fault::Write,
            Fault::Sync,
            Fault::Rename,
        ] {
            let directory = TestDir::new();
            let paths = directory.paths();
            let initial = Store::new(paths.clone());
            initial
                .write_json("fixture.json", &json!({"saved":"synthetic-old"}))
                .unwrap();
            let old = fs::read(paths.data_file("fixture.json")).unwrap();
            let store = Store::with_filesystem(paths.clone(), Arc::new(FaultFs { fault }));
            assert_eq!(
                store.write_json("fixture.json", &json!({"saved":"synthetic-new"})),
                Err(StorageError::WriteFailed)
            );
            assert_eq!(fs::read(paths.data_file("fixture.json")).unwrap(), old);
            assert_eq!(
                fs::read_dir(paths.data_file("fixture.json").parent().unwrap())
                    .unwrap()
                    .count(),
                1
            );
        }
    }

    struct CannotSerialize;
    impl Serialize for CannotSerialize {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom(
                "SYNTHETIC_PRIVATE_SERIALIZATION_DETAIL",
            ))
        }
    }

    #[test]
    fn serialization_and_output_limit_failure_keep_previous_file() {
        let directory = TestDir::new();
        let paths = directory.paths();
        let store = Store::new(paths.clone());
        store
            .write_json("fixture.json", &json!({"saved":"synthetic-old"}))
            .unwrap();
        let previous = fs::read(paths.data_file("fixture.json")).unwrap();
        let error = store
            .write_json("fixture.json", &CannotSerialize)
            .unwrap_err();
        assert_eq!(error, StorageError::WriteFailed);
        assert!(!error
            .response("dashboard")
            .to_string()
            .contains("SYNTHETIC_PRIVATE_SERIALIZATION_DETAIL"));
        assert_eq!(
            store.write_json("fixture.json", &"x".repeat(MAX_FILE_BYTES + 1)),
            Err(StorageError::WriteFailed)
        );
        assert_eq!(fs::read(paths.data_file("fixture.json")).unwrap(), previous);
        assert_eq!(
            fs::read_dir(paths.data_file("fixture.json").parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    struct CollisionFs(Mutex<Option<PathBuf>>);
    impl FileSystem for CollisionFs {
        fn read_bounded(&self, path: &Path, max: usize) -> io::Result<Option<Vec<u8>>> {
            NativeFileSystem.read_bounded(path, max)
        }
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            NativeFileSystem.create_dir_all(path)
        }
        fn create_new(&self, path: &Path) -> io::Result<Box<dyn WritableFile>> {
            fs::write(path, b"synthetic-preexisting-temporary").unwrap();
            *self.0.lock() = Some(path.to_owned());
            NativeFileSystem.create_new(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            NativeFileSystem.rename(from, to)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            NativeFileSystem.remove_file(path)
        }
    }

    #[test]
    fn failed_exclusive_create_never_removes_an_unowned_file() {
        let directory = TestDir::new();
        let filesystem = Arc::new(CollisionFs(Mutex::new(None)));
        let store = Store::with_filesystem(directory.paths(), filesystem.clone());
        assert_eq!(
            store.write_json("fixture.json", &json!({"new":true})),
            Err(StorageError::WriteFailed)
        );
        let collision = filesystem.0.lock().clone().unwrap();
        assert_eq!(
            fs::read(collision).unwrap(),
            b"synthetic-preexisting-temporary"
        );
        assert!(!directory.paths().data_file("fixture.json").exists());
    }

    struct DelayedSerialization {
        ready: mpsc::Sender<()>,
        release: Option<Arc<Barrier>>,
        value: Value,
    }

    impl Serialize for DelayedSerialization {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.ready.send(()).unwrap();
            if let Some(release) = &self.release {
                release.wait();
            }
            self.value.serialize(serializer)
        }
    }

    #[test]
    fn delayed_older_serialization_cannot_overwrite_a_newer_completed_save() {
        let directory = TestDir::new();
        let store = Store::new(directory.paths());
        let (ready, received) = mpsc::channel();
        let release = Arc::new(Barrier::new(2));
        let delayed = DelayedSerialization {
            ready,
            release: Some(release.clone()),
            value: json!({"generation":"old"}),
        };
        let old_store = store.clone();
        let old = std::thread::spawn(move || old_store.write_json("fixture.json", &delayed));
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        store
            .write_json("fixture.json", &json!({"generation":"new"}))
            .unwrap();
        release.wait();
        assert_eq!(old.join().unwrap(), Err(StorageError::Superseded));
        assert_eq!(
            store.read_json("fixture.json").unwrap(),
            Some(json!({"generation":"new"}))
        );
    }

    #[test]
    fn newer_failed_serialization_still_supersedes_older_pending_intent() {
        let directory = TestDir::new();
        let store = Store::new(directory.paths());
        store
            .write_json("fixture.json", &json!({"generation":"previous"}))
            .unwrap();
        let (ready, received) = mpsc::channel();
        let release = Arc::new(Barrier::new(2));
        let delayed = DelayedSerialization {
            ready,
            release: Some(release.clone()),
            value: json!({"generation":"older-pending"}),
        };
        let old_store = store.clone();
        let old = std::thread::spawn(move || old_store.write_json("fixture.json", &delayed));
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            store.write_json("fixture.json", &CannotSerialize),
            Err(StorageError::WriteFailed)
        );
        release.wait();
        assert_eq!(old.join().unwrap(), Err(StorageError::Superseded));
        assert_eq!(
            store.read_json("fixture.json").unwrap(),
            Some(json!({"generation":"previous"}))
        );
    }

    struct DelayedWriteFs(Mutex<Option<(mpsc::Sender<()>, Arc<Barrier>)>>);
    impl FileSystem for DelayedWriteFs {
        fn read_bounded(&self, path: &Path, max: usize) -> io::Result<Option<Vec<u8>>> {
            NativeFileSystem.read_bounded(path, max)
        }
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            NativeFileSystem.create_dir_all(path)
        }
        fn create_new(&self, path: &Path) -> io::Result<Box<dyn WritableFile>> {
            Ok(Box::new(DelayedWrite {
                inner: NativeFileSystem.create_new(path)?,
                delay: self.0.lock().take(),
            }))
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            NativeFileSystem.rename(from, to)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            NativeFileSystem.remove_file(path)
        }
    }
    struct DelayedWrite {
        inner: Box<dyn WritableFile>,
        delay: Option<(mpsc::Sender<()>, Arc<Barrier>)>,
    }
    impl Write for DelayedWrite {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some((ready, release)) = self.delay.take() {
                ready.send(()).unwrap();
                release.wait();
            }
            self.inner.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }
    impl WritableFile for DelayedWrite {
        fn sync_all(&mut self) -> io::Result<()> {
            self.inner.sync_all()
        }
    }

    #[test]
    fn overlapping_writes_keep_the_latest_accepted_value() {
        let directory = TestDir::new();
        let (write_ready, write_received) = mpsc::channel();
        let release = Arc::new(Barrier::new(2));
        let filesystem = Arc::new(DelayedWriteFs(Mutex::new(Some((
            write_ready,
            release.clone(),
        )))));
        let store = Store::with_filesystem(directory.paths(), filesystem);
        let old_store = store.clone();
        let old = std::thread::spawn(move || {
            old_store.write_json("fixture.json", &json!({"generation":"old"}))
        });
        write_received.recv_timeout(Duration::from_secs(5)).unwrap();
        let (ready, received) = mpsc::channel();
        let new_store = store.clone();
        let new = std::thread::spawn(move || {
            new_store.write_json(
                "fixture.json",
                &DelayedSerialization {
                    ready,
                    release: None,
                    value: json!({"generation":"new"}),
                },
            )
        });
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        release.wait();
        assert_eq!(old.join().unwrap(), Err(StorageError::Superseded));
        new.join().unwrap().unwrap();
        assert_eq!(
            store.read_json("fixture.json").unwrap(),
            Some(json!({"generation":"new"}))
        );
        assert_eq!(
            fs::read_dir(
                directory
                    .paths()
                    .data_file("fixture.json")
                    .parent()
                    .unwrap()
            )
            .unwrap()
            .count(),
            1
        );
    }

    #[test]
    fn independent_files_do_not_supersede_each_other() {
        let directory = TestDir::new();
        let store =
            Store::with_filesystem(directory.paths(), Arc::new(FaultFs { fault: Fault::None }));
        let (ready, received) = mpsc::channel();
        let release = Arc::new(Barrier::new(2));
        let old_store = store.clone();
        let old_release = release.clone();
        let settings = std::thread::spawn(move || {
            old_store.write_json(
                "settings.json",
                &DelayedSerialization {
                    ready,
                    release: Some(old_release),
                    value: json!({"default_profile":"synthetic"}),
                },
            )
        });
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        store
            .write_json("dashboard.json", &json!({"version":1,"tiles":[]}))
            .unwrap();
        release.wait();
        settings.join().unwrap().unwrap();
        assert_eq!(
            store.read_json("settings.json").unwrap(),
            Some(json!({"default_profile":"synthetic"}))
        );
    }
}
