//! Explicit, disposable storage for tests. Never changes process environment.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths::AppPaths;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TestDir {
    root: PathBuf,
}

impl TestDir {
    pub(crate) fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..128 {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "cloud-burrito-test-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&root) {
                Ok(()) => return Self { root },
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not create isolated test directory: {error}"),
            }
        }
        panic!("could not allocate a unique test directory");
    }

    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    pub(crate) fn paths(&self) -> AppPaths {
        AppPaths::from_home(self.root.clone())
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // Only remove the unique directory this instance successfully created.
        let _ = fs::remove_dir_all(&self.root);
    }
}

mod tests {
    use super::*;
    use crate::{audit, aws::policy, dashboard, settings};
    use serde_json::json;

    #[test]
    fn directories_are_unique_and_cleaned_up_on_drop() {
        let (left_path, right_path) = {
            let left = TestDir::new();
            let right = TestDir::new();
            assert_ne!(left.path(), right.path());
            assert!(left.path().is_dir());
            assert!(right.path().is_dir());
            (left.path().to_path_buf(), right.path().to_path_buf())
        };
        assert!(!left_path.exists());
        assert!(!right_path.exists());
    }

    #[test]
    fn independent_stores_do_not_leak_settings_dashboard_policy_or_audit() {
        let left_dir = TestDir::new();
        let right_dir = TestDir::new();
        let left = left_dir.paths();
        let right = right_dir.paths();

        settings::save(&left, &json!({"default_profile": "synthetic-left"}));
        assert_eq!(settings::load(&right)["default_profile"], json!(""));
        settings::save(&right, &json!({"default_profile": "synthetic-right"}));
        assert_eq!(
            settings::load(&left)["default_profile"],
            json!("synthetic-left")
        );

        dashboard::save(&left, &json!([{"id": "left-tile"}]));
        assert_eq!(dashboard::load(&right)["tiles"], json!([]));
        dashboard::save(&right, &json!([{"id": "right-tile"}]));
        assert_eq!(
            dashboard::load(&left)["tiles"],
            json!([{"id": "left-tile"}])
        );

        policy::write_text(
            &left,
            "statements:\n  - effect: Allow\n    action: [logs:*]\n",
        )
        .unwrap();
        assert!(!policy::policy_path(&right).exists());
        assert_eq!(
            policy::load(&right)
                .unwrap()
                .decision("cloudformation", "ListStacks"),
            policy::Effect::Allow
        );
        assert_eq!(
            policy::load(&left)
                .unwrap()
                .decision("cloudformation", "ListStacks"),
            policy::Effect::Deny
        );

        audit::append(&left, json!({"store": "left"}), 10.0);
        assert!(audit::tail(&right, 10).is_empty());
        audit::append(&right, json!({"store": "right"}), 20.0);
        assert_eq!(
            audit::tail(&left, 10),
            vec![json!({"store": "left", "ts": 10.0})]
        );
        assert_eq!(
            audit::tail(&right, 10),
            vec![json!({"store": "right", "ts": 20.0})]
        );
    }
}
