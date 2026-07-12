//! App-local filesystem paths.

use std::path::PathBuf;

pub const APP_DATA_DIR: &str = ".cloud_burrito";

pub fn data_file(name: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    home.join(APP_DATA_DIR).join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn data_file_uses_cloud_burrito_dir_only() {
        let _g = crate::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_home = std::env::var_os("HOME");
        let tmp =
            std::env::temp_dir().join(format!("cloud-burrito-path-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HOME", &tmp);

        let path = data_file("settings.json");

        assert_eq!(path, tmp.join(APP_DATA_DIR).join("settings.json"));
        assert!(!path.exists());
        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}
