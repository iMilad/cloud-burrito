//! AWS-facing layer: credential context, config-file parsing, read-only guard.

pub mod config_file;
pub mod context;
pub mod guard;
pub mod policy;

pub use context::{sso_login_required, AwsContext};
