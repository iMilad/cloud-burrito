//! App-local filesystem paths.

use std::path::{Path, PathBuf};

pub const APP_DATA_DIR: &str = ".cloud_burrito";

#[derive(Clone, Debug)]
pub struct AppPaths {
    data_dir: PathBuf,
}

impl AppPaths {
    #[cfg(test)]
    pub fn from_home(home: PathBuf) -> Self {
        Self::try_from_home(Some(home)).expect("application data requires an absolute user home")
    }

    pub fn try_from_home(home: Option<PathBuf>) -> Result<Self, &'static str> {
        let home = home.filter(|home| home.is_absolute()).ok_or(
            "Cloud Burrito could not determine an absolute user home; startup was stopped.",
        )?;
        Ok(Self {
            data_dir: home.join(APP_DATA_DIR),
        })
    }

    pub fn native() -> Result<Self, &'static str> {
        #[cfg(test)]
        panic!("tests must inject AppPaths instead of accessing personal storage");

        #[cfg(not(test))]
        Self::try_from_home(dirs::home_dir())
    }

    pub fn data_file(&self, name: &str) -> PathBuf {
        self.data_dir.join(name)
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

impl Default for AppPaths {
    fn default() -> Self {
        Self::native().expect("application data requires an absolute user home")
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
    fn unavailable_or_relative_home_never_becomes_installation_local_storage() {
        for home in [
            None,
            Some(PathBuf::new()),
            Some(PathBuf::from("relative-home")),
        ] {
            let error = AppPaths::try_from_home(home).unwrap_err();
            assert_eq!(
                error,
                "Cloud Burrito could not determine an absolute user home; startup was stopped."
            );
            assert!(!error.contains("relative-home"));
        }
    }

    #[test]
    fn absolute_home_with_spaces_and_unicode_keeps_the_existing_data_location() {
        let tmp = TestDir::new();
        let home = tmp.path().join("Synthetic Home 雲");
        let paths = AppPaths::try_from_home(Some(home.clone())).unwrap();
        assert_eq!(
            paths.data_file("settings.json"),
            home.join(APP_DATA_DIR).join("settings.json")
        );
        assert!(!home.exists(), "path selection must not create storage");
    }

    #[cfg(windows)]
    #[test]
    fn windows_home_requires_a_complete_drive_or_unc_path() {
        for home in [
            r"C:\Synthetic Home 雲",
            r"\\synthetic-host\synthetic-share\home",
        ] {
            let home = PathBuf::from(home);
            assert_eq!(
                AppPaths::try_from_home(Some(home.clone()))
                    .unwrap()
                    .data_file("settings.json"),
                home.join(APP_DATA_DIR).join("settings.json")
            );
        }
        for home in [r"C:relative-home", r"\root-relative-home"] {
            assert!(AppPaths::try_from_home(Some(PathBuf::from(home))).is_err());
        }
    }

    #[test]
    #[should_panic(expected = "tests must inject AppPaths")]
    fn default_paths_fail_closed_in_tests() {
        let _ = AppPaths::default();
    }
}
