//! Owner-only Unix permissions for app-owned storage, including older files.
//! Windows retains inherited ACLs. Parent paths belong to AppPaths; final
//! symlinks and multiply linked files are rejected before changing permissions.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::path::Path;

use crate::paths::AppPaths;

pub(crate) fn no_follow_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        // Nonblocking also prevents an unexpected FIFO from blocking startup.
        #[cfg(target_os = "macos")]
        options.custom_flags(0x0000_0100 | 0x0000_0004); // O_NOFOLLOW | O_NONBLOCK
        #[cfg(target_os = "linux")]
        options.custom_flags(0x0002_0000 | 0x0000_0800); // O_NOFOLLOW | O_NONBLOCK
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    options
}

fn invalid_path() -> io::Error {
    io::Error::other("Application storage must use ordinary private files and directories")
}

fn secure_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid_path());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0000_0400 != 0 {
            return Err(invalid_path());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = no_follow_options().read(true).open(path)?;
        if !directory.metadata()?.is_dir() {
            return Err(invalid_path());
        }
        directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn directory_builder(recursive: bool) -> DirBuilder {
    let mut builder = DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
}

pub(crate) fn ensure_directory(path: &Path) -> io::Result<()> {
    directory_builder(true).create(path)?;
    secure_directory(path)
}

pub(crate) fn create_directory(path: &Path) -> io::Result<()> {
    directory_builder(false).create(path)?;
    secure_directory(path)
}

pub(crate) fn secure_file(file: &File) -> io::Result<()> {
    if !crate::audit::regular_file(&file.metadata()?) {
        return Err(invalid_path());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if file.metadata()?.nlink() != 1 {
            return Err(invalid_path());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(crate) fn secure_existing_file(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if crate::audit::regular_file(&metadata) => {}
        Ok(_) => return Err(invalid_path()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    secure_file(&no_follow_options().read(true).open(path)?)
}

fn audit_names() -> impl Iterator<Item = String> {
    std::iter::once("audit.log".into())
        .chain((1..crate::audit::RETENTION_FILES).map(|n| format!("audit.{n}")))
}

fn archive_name(name: &str) -> bool {
    name.strip_prefix("audit-preserved-")
        .and_then(|suffix| suffix.split_once('-'))
        .is_some_and(|(epoch, counter)| {
            [epoch, counter]
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
        })
}

/// Run before providers and the audit writer start. Repair permissions only;
/// never rewrite contents, recurse into unrelated folders or follow links.
pub(crate) fn secure_app_storage(paths: &AppPaths) -> io::Result<()> {
    let root = paths.data_dir();
    ensure_directory(root)?;
    for name in ["settings.json", "policy.yaml", "dashboard.json"] {
        secure_existing_file(&root.join(name))?;
    }
    for name in audit_names() {
        secure_existing_file(&root.join(name))?;
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name().to_str().is_some_and(archive_name) {
            let directory = entry.path();
            secure_directory(&directory)?;
            for name in audit_names() {
                secure_existing_file(&directory.join(name))?;
            }
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn legacy_file(path: &Path) {
        fs::write(path, "synthetic existing content").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
    }

    #[test]
    fn startup_repairs_existing_files_and_archives_without_rewriting_content() {
        let tmp = TestDir::new();
        let paths = tmp.paths();
        let root = paths.data_dir();
        let archive = root.join("audit-preserved-abc-1");
        fs::create_dir_all(&archive).unwrap();
        for directory in [root, &archive] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut files: Vec<_> = ["settings.json", "policy.yaml", "dashboard.json"]
            .into_iter()
            .map(|name| root.join(name))
            .collect();
        files.extend(audit_names().flat_map(|name| [root.join(&name), archive.join(name)]));
        for file in &files {
            legacy_file(file);
        }
        let unrelated = root.join("unrelated.keep");
        legacy_file(&unrelated);
        for _ in 0..2 {
            secure_app_storage(&paths).unwrap();
            assert_eq!(mode(root), 0o700);
            assert_eq!(mode(&archive), 0o700);
            for file in &files {
                assert_eq!(mode(file), 0o600);
                assert_eq!(
                    fs::read_to_string(file).unwrap(),
                    "synthetic existing content"
                );
            }
            assert_eq!(mode(&unrelated), 0o644);
        }
    }

    #[test]
    fn startup_rejects_file_links_without_changing_the_external_target() {
        for hard_link in [false, true] {
            let tmp = TestDir::new();
            let paths = tmp.paths();
            ensure_directory(paths.data_dir()).unwrap();
            let outside = tmp.path().join("outside.txt");
            legacy_file(&outside);
            if hard_link {
                fs::hard_link(&outside, paths.data_file("policy.yaml")).unwrap();
            } else {
                symlink(&outside, paths.data_file("policy.yaml")).unwrap();
            }
            assert!(secure_app_storage(&paths).is_err());
            assert_eq!(mode(&outside), 0o644);
            assert_eq!(
                fs::read_to_string(outside).unwrap(),
                "synthetic existing content"
            );
        }
    }

    #[test]
    fn startup_rejects_linked_storage_and_archive_directories() {
        for archive in [false, true] {
            let tmp = TestDir::new();
            let paths = tmp.paths();
            let outside = tmp.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::set_permissions(&outside, fs::Permissions::from_mode(0o755)).unwrap();
            let destination = if archive {
                ensure_directory(paths.data_dir()).unwrap();
                paths.data_file("audit-preserved-abc-1")
            } else {
                paths.data_dir().to_path_buf()
            };
            symlink(&outside, destination).unwrap();
            assert!(secure_app_storage(&paths).is_err());
            assert_eq!(mode(&outside), 0o755);
        }
    }

    #[test]
    fn new_storage_and_audit_preservation_keep_private_permissions() {
        let tmp = TestDir::new();
        let paths = tmp.paths();
        let store = crate::storage::Store::new(paths.clone());
        store
            .write_json("settings.json", &serde_json::json!({}))
            .unwrap();
        crate::audit::append(&paths, serde_json::json!({"event":"fixture"}), 1.0).unwrap();
        assert_eq!(mode(paths.data_dir()), 0o700);
        assert_eq!(mode(&paths.data_file("settings.json")), 0o600);
        assert_eq!(mode(&paths.data_file("audit.log")), 0o600);
        legacy_file(&paths.data_file("audit.1"));
        let preserved = crate::audit::preserve_history(&paths).unwrap();
        let archive = Path::new(preserved["preserved_location"].as_str().unwrap());
        assert_eq!(mode(archive), 0o700);
        assert_eq!(mode(&archive.join("audit.log")), 0o600);
        assert_eq!(mode(&archive.join("audit.1")), 0o600);
        legacy_file(&paths.data_file("audit.log"));
        crate::audit::append(&paths, serde_json::json!({"event":"fixture"}), 2.0).unwrap();
        assert_eq!(mode(&paths.data_file("audit.log")), 0o600);
    }
}
