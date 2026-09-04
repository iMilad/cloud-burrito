//! App-local filesystem paths.

use std::path::PathBuf;

pub const APP_DATA_DIR: &str = ".cloud_burrito";

#[derive(Clone, Debug)]
pub struct AppPaths {
    data_dir: PathBuf,
}

impl AppPaths {
    pub fn from_home(home: PathBuf) -> Self {
        Self {
            data_dir: home.join(APP_DATA_DIR),
        }
    }

    pub fn data_file(&self, name: &str) -> PathBuf {
        self.data_dir.join(name)
    }
}

impl Default for AppPaths {
    fn default() -> Self {
        #[cfg(test)]
        panic!("tests must inject AppPaths instead of accessing personal storage");

        #[cfg(not(test))]
        Self::from_home(dirs::home_dir().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn data_file_uses_cloud_burrito_dir_only() {
        let tmp = TestDir::new();
        let path = tmp.paths().data_file("settings.json");

        assert_eq!(path, tmp.path().join(APP_DATA_DIR).join("settings.json"));
        assert!(!path.exists());
    }

    #[test]
    #[should_panic(expected = "tests must inject AppPaths")]
    fn default_paths_fail_closed_in_tests() {
        let _ = AppPaths::default();
    }
}
