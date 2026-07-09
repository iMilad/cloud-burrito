//! App-local filesystem paths and legacy data migration.

use std::fs;
use std::path::{Path, PathBuf};

pub const APP_DATA_DIR: &str = ".cloud_burrito";
pub const LEGACY_APP_DATA_DIR: &str = ".aws_control_center";

pub fn data_file(name: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    let path = home.join(APP_DATA_DIR).join(name);
    migrate_legacy_file(&home, name, &path);
    path
}

fn migrate_legacy_file(home: &Path, name: &str, path: &Path) {
    if path.exists() {
        return;
    }
    let legacy = home.join(LEGACY_APP_DATA_DIR).join(name);
    if !legacy.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::copy(legacy, path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_file_migrates_legacy_file_to_cloud_burrito_dir() {
        let _g = crate::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_home = std::env::var_os("HOME");
        let tmp = std::env::temp_dir().join(format!("cloud-burrito-path-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let legacy_dir = tmp.join(LEGACY_APP_DATA_DIR);
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(legacy_dir.join("settings.json"), "{\"default_region\":\"eu-west-1\"}").unwrap();
        std::env::set_var("HOME", &tmp);

        let path = data_file("settings.json");

        assert_eq!(path, tmp.join(APP_DATA_DIR).join("settings.json"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"default_region\":\"eu-west-1\"}"
        );
        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}
