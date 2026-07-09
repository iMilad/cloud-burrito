//! Read SSO-relevant profile metadata from an AWS config file (`~/.aws/config`).
//!
//! Port of the Python sidecar's `aws_config.py`. Powers the topbar account
//! picker (`aws.listProfiles`) and the cached-SSO-token expiry lookup shown in
//! the auth pill / Identity panel.

use std::fs;
use std::path::PathBuf;

use ini::Ini;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};

/// Expand a leading `~` to the user's home directory.
pub fn expand(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Read profiles plus diagnostics so the UI can explain an empty list (wrong
/// path, missing file, parse error). Matches the Python `inspect()` shape.
pub fn inspect(config_path: &str) -> Value {
    let resolved = expand(config_path);
    let file_exists = resolved.is_file();
    let mut info = json!({
        "config_path": config_path,
        "resolved_path": resolved.to_string_lossy(),
        "file_exists": file_exists,
        "home": std::env::var("HOME").unwrap_or_default(),
        "error": Value::Null,
        "profiles": [],
    });
    if !file_exists {
        return info;
    }
    match Ini::load_from_file(&resolved) {
        Ok(ini) => info["profiles"] = Value::Array(list_profiles(&ini)),
        Err(e) => info["error"] = json!(format!("ParseError: {e}")),
    }
    info
}

fn row(name: &str, props: &ini::Properties) -> Value {
    let g = |k: &str| props.get(k).unwrap_or("").to_string();
    json!({
        "name": name,
        "account_id": g("sso_account_id"),
        "role_name": g("sso_role_name"),
        "region": g("region"),
        "sso_session": g("sso_session"),
        // Legacy SSO profiles inline the start_url + sso_region instead of
        // pointing at a [sso-session] block. Expose both shapes.
        "sso_start_url": g("sso_start_url"),
        "sso_region": g("sso_region"),
    })
}

fn list_profiles(ini: &Ini) -> Vec<Value> {
    let mut out = Vec::new();
    for (section, props) in ini.iter() {
        let section = match section {
            Some(s) => s,
            None => continue, // nameless general section
        };
        let name = if section == "default" {
            "default".to_string()
        } else if let Some(rest) = section.strip_prefix("profile ") {
            rest.trim().to_string()
        } else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        out.push(row(&name, props));
    }
    out
}

/// Path to the cached SSO token for a session, keyed by `sha1(session_name)`
/// like the AWS CLI / botocore.
pub fn sso_cache_path(sso_session_name: &str) -> PathBuf {
    let mut hasher = Sha1::new();
    hasher.update(sso_session_name.as_bytes());
    let sha = hex::encode(hasher.finalize());
    expand(&format!("~/.aws/sso/cache/{sha}.json"))
}

/// Read `{cache_path, exists, expires_at, error}` for an SSO session's cached
/// token. `expires_at` is the ISO-8601 string the CLI wrote (or null).
pub fn read_sso_token(sso_session_name: &str) -> Value {
    let p = sso_cache_path(sso_session_name);
    let exists = p.is_file();
    let mut out = json!({
        "cache_path": p.to_string_lossy(),
        "exists": exists,
        "expires_at": Value::Null,
        "error": Value::Null,
    });
    if !exists {
        return out;
    }
    match fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(data) => {
            out["expires_at"] = data
                .get("expiresAt")
                .or_else(|| data.get("expires_at"))
                .cloned()
                .unwrap_or(Value::Null);
        }
        None => out["error"] = json!("could not read/parse SSO token cache"),
    }
    out
}
