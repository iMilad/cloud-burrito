//! Read SSO-relevant profile metadata from an AWS config file (`~/.aws/config`).
//!
//! Powers account/profile discovery and pure validation of the selected SSO
//! configuration. Verified credential expiry is reported by the context layer.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use ini::Ini;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};

/// Explicit settings captured by the caller before selecting a provider.
/// Neither this selection nor the parser consults process environment variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SsoProfileSelection {
    pub config_path: PathBuf,
    pub profile: String,
    pub expected_account_id: String,
    pub requested_region: String,
    pub session_override: Option<String>,
    pub settings_revision: u64,
}

/// Owned, validated inputs for one direct SSO provider. No original config text,
/// credentials, executable instructions, or fallback profile is retained.
/// Compare the complete snapshot for provider/cache identity; the revision alone
/// only identifies the caller's settings generation, not a config-file revision.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SsoProfileSnapshot {
    pub config_path: PathBuf,
    pub profile: String,
    pub account_id: String,
    pub role_name: String,
    pub region: String,
    pub sso_region: String,
    pub start_url: String,
    pub session_name: Option<String>,
    pub registration_scopes: Option<String>,
    pub settings_revision: u64,
}

impl SsoProfileSnapshot {
    /// Parse only the selected profile and its explicitly referenced SSO session.
    /// Unrelated/default profiles never supply missing values. This function is
    /// pure: callers provide file contents, path, account, region and revision.
    pub fn parse(text: &str, selection: &SsoProfileSelection) -> Result<Self, String> {
        validate_selection(selection)?;
        let ini = Ini::load_from_str_noescape(text)
            .map_err(|_| "selected AWS configuration is not valid INI".to_string())?;
        Self::from_sections(&ConfigSections::new(&ini), selection)
    }

    fn from_sections(
        sections: &ConfigSections<'_>,
        selection: &SsoProfileSelection,
    ) -> Result<Self, String> {
        let profile =
            selected_properties(sections, &selection.profile, "selected SSO profile", false)?;
        let session_name = optional(&profile, "sso_session")?.map(str::to_string);
        if let Some(name) = session_name.as_deref() {
            validate_name(name, "SSO session name")?;
        }
        if let Some(selected_session) = selection
            .session_override
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        {
            validate_name(selected_session, "configured SSO session")?;
            if session_name.as_deref() != Some(selected_session) {
                return Err("configured SSO session conflicts with the selected profile".into());
            }
        }
        let session = match session_name.as_deref() {
            Some(name) => Some(selected_properties(
                sections,
                name,
                "referenced SSO session",
                true,
            )?),
            None => None,
        };
        let account_id = required(&profile, "sso_account_id")?.to_string();
        validate_account(&account_id)?;
        if account_id != selection.expected_account_id {
            return Err("selected profile account does not match the requested account".into());
        }
        let role_name = required(&profile, "sso_role_name")?.to_string();
        validate_name(&role_name, "SSO role name")?;
        if let Some(region) = optional(&profile, "region")? {
            validate_region(region, "profile region")?;
        }
        let start_url = resolved_sso_field(&profile, session.as_ref(), "sso_start_url", true)?
            .expect("required SSO field checked above");
        validate_start_url(&start_url)?;
        let sso_region = resolved_sso_field(&profile, session.as_ref(), "sso_region", true)?
            .expect("required SSO field checked above");
        validate_region(&sso_region, "SSO region")?;
        let registration_scopes =
            resolved_sso_field(&profile, session.as_ref(), "sso_registration_scopes", false)?;
        Ok(Self {
            config_path: selection.config_path.clone(),
            profile: selection.profile.clone(),
            account_id,
            role_name,
            region: selection.requested_region.clone(),
            sso_region,
            start_url,
            session_name,
            registration_scopes,
            settings_revision: selection.settings_revision,
        })
    }

    /// The CLI cache uses the named session, or the start URL for legacy SSO.
    /// The caller supplies the cache directory; this does not discover a home.
    pub fn token_cache_key(&self) -> &str {
        self.session_name.as_deref().unwrap_or(&self.start_url)
    }
}

type ConfigProperties = BTreeMap<String, String>;

fn validate_selection(selection: &SsoProfileSelection) -> Result<(), String> {
    if selection.config_path.as_os_str().is_empty() {
        return Err("an explicit AWS configuration path is required".into());
    }
    validate_name(&selection.profile, "selected profile")?;
    validate_account(&selection.expected_account_id)?;
    validate_region(&selection.requested_region, "selected region")
}

/// Index logical aliases once. Duplicate sections remain distinct and invalid;
/// discovery and verification use exactly the same selection rules.
struct ConfigSections<'a> {
    profiles: BTreeMap<&'a str, Vec<&'a ini::Properties>>,
    sessions: BTreeMap<&'a str, Vec<&'a ini::Properties>>,
}

impl<'a> ConfigSections<'a> {
    fn new(ini: &'a Ini) -> Self {
        let mut sections = Self {
            profiles: BTreeMap::new(),
            sessions: BTreeMap::new(),
        };
        for (section, properties) in ini.iter() {
            let Some(section) = section else {
                continue;
            };
            if let Some(name) = logical_section_name(section, false) {
                sections.profiles.entry(name).or_default().push(properties);
            } else if let Some(name) = logical_section_name(section, true) {
                sections.sessions.entry(name).or_default().push(properties);
            }
        }
        sections
    }
}

/// Match the SDK's section-prefix whitespace normalization. Both default
/// spellings identify the same profile, but selection rejects duplicates rather
/// than applying the SDK's profile merging or default-profile precedence.
fn logical_section_name(section: &str, session: bool) -> Option<&str> {
    let section = section.trim_matches([' ', '\t']);
    if !session && section == "default" {
        return Some("default");
    }
    let (prefix, name) = section.split_once([' ', '\t'])?;
    let expected_prefix = if session { "sso-session" } else { "profile" };
    let name = name.trim();
    (prefix.trim() == expected_prefix && !name.is_empty()).then_some(name)
}

fn selected_properties(
    sections: &ConfigSections<'_>,
    name: &str,
    description: &str,
    session: bool,
) -> Result<ConfigProperties, String> {
    let index = if session {
        &sections.sessions
    } else {
        &sections.profiles
    };
    let selected_sections = index
        .get(name)
        .ok_or_else(|| format!("{description} is missing from the selected configuration"))?;
    if selected_sections.len() != 1 {
        return Err(format!("{description} has ambiguous duplicate sections"));
    }
    let mut selected = BTreeMap::new();
    for (key, value) in selected_sections[0].iter() {
        let key = key.trim().to_ascii_lowercase();
        if !supported_setting(&key, session) {
            return Err(format!(
                "{description} contains an unsupported setting; only direct SSO configuration is supported"
            ));
        }
        if value.chars().any(char::is_control) {
            return Err(format!(
                "{description} contains a multiline or control-character value"
            ));
        }
        if selected.insert(key, value.trim().to_string()).is_some() {
            return Err(format!("{description} has duplicate settings"));
        }
    }
    Ok(selected)
}

fn supported_setting(key: &str, session: bool) -> bool {
    matches!(
        key,
        "sso_start_url" | "sso_region" | "sso_registration_scopes"
    ) || (!session
        && matches!(
            key,
            "sso_session" | "sso_account_id" | "sso_role_name" | "region"
            // These inert values are never forwarded to a provider or child.
            | "output" | "cli_pager" | "cli_auto_prompt" | "cli_history"
            | "cli_timestamp_format" | "cli_binary_format" | "retry_mode"
            | "max_attempts" | "parameter_validation" | "tcp_keepalive"
        ))
}

fn optional<'a>(properties: &'a ConfigProperties, key: &str) -> Result<Option<&'a str>, String> {
    match properties.get(key) {
        Some(value) if value.is_empty() => Err(format!("SSO configuration field {key} is empty")),
        value => Ok(value.map(String::as_str)),
    }
}

fn required<'a>(properties: &'a ConfigProperties, key: &str) -> Result<&'a str, String> {
    optional(properties, key)?.ok_or_else(|| format!("SSO configuration requires {key}"))
}

fn resolved_sso_field(
    profile: &ConfigProperties,
    session: Option<&ConfigProperties>,
    key: &str,
    required: bool,
) -> Result<Option<String>, String> {
    let inline = optional(profile, key)?;
    let session_value = session.map(|s| optional(s, key)).transpose()?.flatten();
    if matches!((inline, session_value), (Some(a), Some(b)) if a != b) {
        return Err(format!(
            "profile and SSO session have conflicting {key} values"
        ));
    }
    // A referenced session must be complete itself; inline fields cannot patch
    // an incomplete session or redirect its provider settings.
    let resolved = if session.is_some() {
        session_value
    } else {
        inline
    };
    if required && resolved.is_none() {
        return Err(format!("SSO configuration requires {key}"));
    }
    Ok(resolved.map(str::to_string))
}

fn validate_name(value: &str, description: &str) -> Result<(), String> {
    if value.is_empty()
        || value.trim() != value
        || value
            .chars()
            .any(|c| c.is_control() || c == '[' || c == ']')
    {
        return Err(format!("{description} is invalid"));
    }
    Ok(())
}

fn validate_account(value: &str) -> Result<(), String> {
    if value.len() != 12 || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err("SSO account must contain exactly 12 digits".into());
    }
    Ok(())
}

fn validate_region(value: &str, description: &str) -> Result<(), String> {
    let segments: Vec<_> = value.split('-').collect();
    if segments.len() < 3
        || segments.iter().any(|segment| segment.is_empty())
        || !value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        || !segments
            .last()
            .is_some_and(|segment| segment.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err(format!("{description} is invalid"));
    }
    Ok(())
}

fn validate_start_url(value: &str) -> Result<(), String> {
    let authority = value
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .filter(|host| !host.is_empty());
    if value
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        || !authority.is_some_and(|host| {
            host.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        })
    {
        return Err("SSO start URL must be an absolute HTTPS URL without user information".into());
    }
    Ok(())
}

/// Expand a leading `~` without falling back to the launch directory.
pub fn expand(path: &str) -> Result<PathBuf, String> {
    expand_with_home(path, cfg!(windows), || {
        #[cfg(test)]
        panic!("tests must inject home expansion instead of accessing personal storage");
        #[cfg(not(test))]
        dirs::home_dir()
    })
}

fn expand_with_home(
    path: &str,
    windows: bool,
    home: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf, String> {
    let rest = if path == "~" {
        Some("")
    } else {
        path.strip_prefix("~/")
            .or_else(|| windows.then(|| path.strip_prefix("~\\")).flatten())
    };
    let Some(rest) = rest else {
        // Explicit custom paths retain their existing spelling and semantics.
        return Ok(PathBuf::from(path));
    };
    let home = home().filter(|home| home.is_absolute()).ok_or_else(|| {
        "User home is unavailable; choose an absolute AWS config path in Settings".to_string()
    })?;
    Ok(if windows {
        home.join(rest.replace('\\', "/"))
    } else {
        home.join(rest)
    })
}

const MAX_CONFIG_BYTES: usize = 2 * 1024 * 1024;
const MAX_DISCOVERY_PROFILES: usize = 500;

#[derive(Clone, Copy)]
enum DiscoveryFailure {
    HomeUnavailable,
    Missing,
    Unreadable,
    TooLarge,
    InvalidUtf8,
    InvalidIni,
}

fn read_failure(error: io::Error) -> DiscoveryFailure {
    if error.kind() == io::ErrorKind::NotFound {
        DiscoveryFailure::Missing
    } else {
        DiscoveryFailure::Unreadable
    }
}

fn read_config_file(path: &Path) -> Result<Vec<u8>, DiscoveryFailure> {
    // Reject directories/devices/FIFOs before opening, then recheck the handle.
    // Only the caller-selected regular config file belongs to discovery.
    if !std::fs::metadata(path).map_err(read_failure)?.is_file() {
        return Err(DiscoveryFailure::Unreadable);
    }
    let file = std::fs::File::open(path).map_err(read_failure)?;
    if !file
        .metadata()
        .map_err(|_| DiscoveryFailure::Unreadable)?
        .is_file()
    {
        return Err(DiscoveryFailure::Unreadable);
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DiscoveryFailure::Unreadable)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(DiscoveryFailure::TooLarge);
    }
    Ok(bytes)
}

/// Discovery reads only the selected configuration file. It never opens token
/// caches, credentials files, provider processes, or service transports.
pub fn inspect(config_path: &str, sso_constraint: Option<&str>) -> Value {
    inspect_with_reader(config_path, sso_constraint, read_config_file)
}

fn inspect_with_reader(
    config_path: &str,
    sso_constraint: Option<&str>,
    read: impl FnOnce(&Path) -> Result<Vec<u8>, DiscoveryFailure>,
) -> Value {
    inspect_resolved(config_path, sso_constraint, expand(config_path), read)
}

fn inspect_resolved(
    config_path: &str,
    sso_constraint: Option<&str>,
    resolved: Result<PathBuf, String>,
    read: impl FnOnce(&Path) -> Result<Vec<u8>, DiscoveryFailure>,
) -> Value {
    let mut info = json!({
        "ok": true,
        "config_path": config_path,
        "resolved_path": resolved.as_ref().ok().map(|path| path.to_string_lossy()),
        "file_exists": true,
        "error": Value::Null,
        "discovery_state": "no_profiles",
        "discovery_reason": Value::Null,
        "profiles": [],
        "partial": false,
        "coverage": {"complete":true, "returned":0, "limit":MAX_DISCOVERY_PROFILES, "omitted":0},
    });
    let result = resolved.map_err(|_| DiscoveryFailure::HomeUnavailable).and_then(|resolved| {
        let bytes = read(&resolved)?;
        if bytes.len() > MAX_CONFIG_BYTES { return Err(DiscoveryFailure::TooLarge); }
        let text = std::str::from_utf8(&bytes).map_err(|_| DiscoveryFailure::InvalidUtf8)?;
        let ini = Ini::load_from_str_noescape(text).map_err(|_| DiscoveryFailure::InvalidIni)?;
        let sections = ConfigSections::new(&ini);
        let profiles: Vec<_> = sections.profiles.iter().take(MAX_DISCOVERY_PROFILES)
            .map(|(name, _)| discovery_row(name, &sections, &resolved, sso_constraint))
            .collect();
        let omitted = sections.profiles.len().saturating_sub(profiles.len());
        info["discovery_state"] = json!(if profiles.is_empty() { "no_profiles" } else { "ready" });
        info["partial"] = json!(omitted > 0);
        info["coverage"] = json!({"complete":omitted == 0, "returned":profiles.len(), "limit":MAX_DISCOVERY_PROFILES, "omitted":omitted});
        info["profiles"] = json!(profiles);
        Ok(())
    });
    if let Err(failure) = result {
        let (state, reason, exists, error) = match failure {
            DiscoveryFailure::HomeUnavailable => (
                "unreadable_config",
                "home_unavailable",
                Value::Null,
                "User home is unavailable. Choose an absolute AWS config path in Settings.",
            ),
            DiscoveryFailure::Missing => (
                "missing_config",
                "not_found",
                json!(false),
                "Selected AWS configuration was not found. Choose a config file in Settings.",
            ),
            DiscoveryFailure::Unreadable => (
                "unreadable_config",
                "not_readable",
                Value::Null,
                "Selected AWS configuration could not be read. Choose a readable regular file.",
            ),
            DiscoveryFailure::TooLarge => (
                "malformed_config",
                "file_too_large",
                json!(true),
                "Selected AWS configuration exceeds the 2 MiB discovery limit",
            ),
            DiscoveryFailure::InvalidUtf8 => (
                "malformed_config",
                "invalid_utf8",
                json!(true),
                "Selected AWS configuration must contain valid UTF-8 text",
            ),
            DiscoveryFailure::InvalidIni => (
                "malformed_config",
                "invalid_ini",
                json!(true),
                "Selected AWS configuration could not be read or parsed",
            ),
        };
        info["ok"] = json!(false);
        info["discovery_state"] = json!(state);
        info["discovery_reason"] = json!(reason);
        info["file_exists"] = exists;
        info["error"] = json!(error);
        info["coverage"] = json!({"complete":false, "returned":0, "limit":MAX_DISCOVERY_PROFILES, "omitted":Value::Null});
    }
    info
}

fn raw_value<'a>(properties: &'a ini::Properties, key: &str) -> Option<&'a str> {
    let mut matches = properties
        .iter()
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case(key));
    let value = matches.next()?.1.trim();
    matches.next().is_none().then_some(value)
}

fn discovery_row(
    name: &str,
    sections: &ConfigSections<'_>,
    path: &Path,
    constraint: Option<&str>,
) -> Value {
    let candidates = &sections.profiles[name];
    let properties = (candidates.len() == 1).then_some(candidates[0]);
    let get = |key| {
        properties
            .and_then(|properties| raw_value(properties, key))
            .unwrap_or("")
    };
    let safe = |value: &str, limit| value.len() <= limit && !value.chars().any(char::is_control);
    let metadata_valid = safe(name, 256)
        && validate_name(name, "profile name").is_ok()
        && safe(get("sso_role_name"), 256)
        && safe(get("region"), 128)
        && safe(get("sso_session"), 256);
    let selection = SsoProfileSelection {
        config_path: path.to_path_buf(),
        profile: name.into(),
        expected_account_id: get("sso_account_id").into(),
        // Resource-region selection remains independent of SSO eligibility.
        // The selected profile's own region is still checked by the parser.
        requested_region: crate::settings::ALLOWED_REGIONS[0].into(),
        session_override: constraint
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string),
        settings_revision: 0,
    };
    let valid = metadata_valid
        && validate_selection(&selection)
            .and_then(|()| SsoProfileSnapshot::from_sections(sections, &selection).map(|_| ()))
            .is_ok();
    let unsupported = properties.is_some_and(|properties| {
        properties
            .iter()
            .any(|(key, _)| !supported_setting(&key.trim().to_ascii_lowercase(), false))
            || raw_value(properties, "sso_session")
                .and_then(|session| sections.sessions.get(session))
                .filter(|sessions| sessions.len() == 1)
                .is_some_and(|sessions| {
                    sessions[0]
                        .iter()
                        .any(|(key, _)| !supported_setting(&key.trim().to_ascii_lowercase(), true))
                })
    });
    let (eligibility, reason) = if valid {
        ("supported_sso", Value::Null)
    } else if unsupported {
        ("unsupported_credentials", json!("Only direct SSO profiles are supported; credential and endpoint overrides are unavailable"))
    } else {
        ("invalid_sso", json!("Selected profile has incomplete, ambiguous, conflicting, or unsupported SSO metadata"))
    };
    let mut row = json!({"name":if safe(name, 256) && validate_name(name, "profile name").is_ok() {name} else {"(invalid profile name)"},
        "eligibility":eligibility, "eligibility_reason":reason});
    for (output, input, limit) in [
        ("account_id", "sso_account_id", 12),
        ("role_name", "sso_role_name", 256),
        ("region", "region", 128),
        ("sso_session", "sso_session", 256),
    ] {
        let value = get(input);
        let valid = safe(value, limit)
            && match input {
                "sso_account_id" => validate_account(value).is_ok(),
                "region" => value.is_empty() || validate_region(value, "profile region").is_ok(),
                _ => value.is_empty() || validate_name(value, "profile metadata").is_ok(),
            };
        row[output] = json!(if valid { value } else { "" });
    }
    row
}

/// Path to the cached SSO token for a session, keyed by `sha1(session_name)`
/// like the AWS CLI / botocore.
pub fn sso_cache_path(sso_session_name: &str) -> Result<PathBuf, String> {
    let mut hasher = Sha1::new();
    hasher.update(sso_session_name.as_bytes());
    let sha = hex::encode(hasher.finalize());
    expand(&format!("~/.aws/sso/cache/{sha}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expansion_uses_only_an_injected_absolute_home() {
        let dir = crate::test_support::TestDir::new();
        let home = dir.path().join("Synthetic Home 雲");
        for (path, windows) in [("~", false), ("~", true)] {
            assert_eq!(
                expand_with_home(path, windows, || Some(home.clone())).unwrap(),
                home
            );
        }
        for (path, windows) in [
            ("~/.aws/config", false),
            ("~/.aws/config", true),
            (r"~\.aws\config", true),
        ] {
            assert_eq!(
                expand_with_home(path, windows, || Some(home.clone())).unwrap(),
                home.join(".aws/config")
            );
        }
        for home in [
            None,
            Some(PathBuf::new()),
            Some(PathBuf::from("synthetic-relative-home")),
        ] {
            let error = expand_with_home("~/.aws/config", false, || home).unwrap_err();
            assert_eq!(
                error,
                "User home is unavailable; choose an absolute AWS config path in Settings"
            );
        }
        assert!(!home.exists());
    }

    #[test]
    fn explicit_custom_config_paths_do_not_resolve_a_personal_home() {
        for (path, windows) in [
            (r"C:\Synthetic Cloud 雲\config.ini", true),
            (r"\\synthetic-host\synthetic-share\config.ini", true),
            ("synthetic-config.ini", false),
            (r"~\.aws\config", false),
        ] {
            let expanded =
                expand_with_home(path, windows, || panic!("unexpected home lookup")).unwrap();
            assert_eq!(expanded, PathBuf::from(path));
        }
    }

    #[test]
    fn missing_home_discovery_reports_no_read_or_false_missing_file() {
        let result = inspect_resolved(
            "~/.aws/config",
            None,
            expand_with_home("~/.aws/config", false, || None),
            |_| panic!("failed path expansion must not open a file"),
        );
        assert_eq!(result["ok"], false);
        assert_eq!(result["discovery_state"], "unreadable_config");
        assert_eq!(result["discovery_reason"], "home_unavailable");
        assert!(result["resolved_path"].is_null());
        assert!(result["file_exists"].is_null());
        assert_eq!(result["coverage"]["complete"], false);
    }

    #[test]
    fn profile_inspection_does_not_echo_parser_content_or_home() {
        let dir = crate::test_support::TestDir::new();
        let path = dir.path().join("synthetic-config.ini");
        std::fs::write(&path, "[synthetic-private-config-marker\n").unwrap();
        let result = inspect(path.to_str().unwrap(), None);
        assert!(result["profiles"].as_array().unwrap().is_empty());
        assert_eq!(
            result["error"],
            "Selected AWS configuration could not be read or parsed"
        );
        assert!(result.get("home").is_none());
        assert!(!result
            .to_string()
            .contains("synthetic-private-config-marker"));
    }

    fn discover_text(text: &str, constraint: Option<&str>) -> Value {
        let text = text.replace("SYNTHETIC_ACCOUNT", &synthetic_account(1));
        inspect_with_reader(
            "synthetic-config.ini",
            constraint,
            |_| Ok(text.into_bytes()),
        )
    }

    #[test]
    fn discovery_distinguishes_missing_unreadable_and_malformed_without_source_errors() {
        let dir = crate::test_support::TestDir::new();
        let path = dir.path().join("synthetic-config.ini");
        let missing = inspect(path.to_str().unwrap(), None);
        assert_eq!(missing["discovery_state"], "missing_config");
        assert_eq!(missing["file_exists"], false);
        let directory = inspect(dir.path().to_str().unwrap(), None);
        assert_eq!(directory["discovery_state"], "unreadable_config");
        let denied = inspect_with_reader("synthetic-config.ini", None, |_| {
            Err(read_failure(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "synthetic-private-permission-marker",
            )))
        });
        assert_eq!(denied["discovery_state"], "unreadable_config");
        assert!(!denied
            .to_string()
            .contains("synthetic-private-permission-marker"));
        for (bytes, reason) in [
            (
                b"[synthetic-private-malformed-marker".to_vec(),
                "invalid_ini",
            ),
            (vec![0xff, 0xfe], "invalid_utf8"),
            (vec![b' '; MAX_CONFIG_BYTES + 1], "file_too_large"),
        ] {
            std::fs::write(&path, bytes).unwrap();
            let result = inspect(path.to_str().unwrap(), None);
            assert_eq!(result["discovery_state"], "malformed_config");
            assert_eq!(result["discovery_reason"], reason);
            assert_eq!(result["profiles"], json!([]));
            assert_eq!(result["coverage"]["complete"], false);
            assert!(!result
                .to_string()
                .contains("synthetic-private-malformed-marker"));
        }
    }

    #[test]
    fn discovery_empty_and_exact_byte_boundary_do_not_claim_profiles() {
        for text in [
            "",
            "# synthetic comment\n",
            "[sso-session unused]\nsso_region=us-east-1\n",
        ] {
            let result = discover_text(text, None);
            assert_eq!(result["discovery_state"], "no_profiles");
            assert_eq!(result["ok"], true);
            assert_eq!(result["coverage"]["complete"], true);
        }
        let dir = crate::test_support::TestDir::new();
        let path = dir.path().join("synthetic-boundary.ini");
        std::fs::write(&path, vec![b' '; MAX_CONFIG_BYTES]).unwrap();
        assert_eq!(
            inspect(path.to_str().unwrap(), None)["discovery_state"],
            "no_profiles"
        );
    }

    #[test]
    fn discovery_reuses_inline_named_session_and_logical_alias_rules() {
        for text in [
            INLINE.to_owned(),
            SESSION.to_owned(),
            INLINE.replace("[profile DemoA]", "[profile\tDemoA]"),
            SESSION.replace(
                "[sso-session demo-session]",
                "[ sso-session\t demo-session ]",
            ),
            INLINE.replace("[profile DemoA]", "[profile   default]"),
            INLINE.replace("[profile DemoA]", "[default]"),
        ] {
            let result = discover_text(&text, None);
            assert_eq!(result["discovery_state"], "ready");
            assert_eq!(result["profiles"].as_array().unwrap().len(), 1);
            assert_eq!(
                result["profiles"][0]["eligibility"], "supported_sso",
                "{result}"
            );
            assert_eq!(result["profiles"][0]["account_id"], synthetic_account(1));
            assert_eq!(result["profiles"][0]["region"], "us-east-1");
            assert_eq!(result["profiles"][0]["eligibility_reason"], Value::Null);
        }
    }

    #[test]
    fn discovery_applies_saved_session_constraint_without_provider_fallback() {
        assert_eq!(
            discover_text(SESSION, Some("demo-session"))["profiles"][0]["eligibility"],
            "supported_sso"
        );
        for text in [SESSION, INLINE] {
            let result = discover_text(text, Some("synthetic-different-session"));
            assert_eq!(result["profiles"][0]["eligibility"], "invalid_sso");
        }
        // A region outside beta remains visible for the explicit region picker.
        let outside_beta = discover_text(
            &INLINE.replace("region = us-east-1", "region = ap-south-1"),
            None,
        );
        assert_eq!(outside_beta["profiles"][0]["region"], "ap-south-1");
        assert_eq!(outside_beta["profiles"][0]["eligibility"], "supported_sso");
    }

    #[test]
    fn discovery_classifies_unsupported_providers_without_exposing_their_values() {
        for key in [
            "credential_process",
            "aws_secret_access_key",
            "source_profile",
            "web_identity_token_file",
            "endpoint_url",
            "services",
        ] {
            for input in [INLINE, SESSION] {
                let source = format!("{input}{key}=synthetic-private-provider-marker\n");
                let result = discover_text(&source, None);
                assert_eq!(
                    result["profiles"][0]["eligibility"], "unsupported_credentials",
                    "{key}"
                );
                assert!(!result
                    .to_string()
                    .contains("synthetic-private-provider-marker"));
            }
        }
        let static_only = discover_text(
            "[profile StaticOnly]\naws_access_key_id=synthetic-private-key-marker\n",
            None,
        );
        assert_eq!(
            static_only["profiles"][0]["eligibility"],
            "unsupported_credentials"
        );
        assert_eq!(static_only["profiles"][0]["account_id"], "");
        assert!(!static_only
            .to_string()
            .contains("synthetic-private-key-marker"));
    }

    #[test]
    fn discovery_duplicate_aliases_and_incomplete_sso_are_not_eligible() {
        for text in [
            format!(
                "{INLINE}\n{}",
                INLINE.replace("[profile DemoA]", "[profile\tDemoA]")
            ),
            format!("{INLINE}SSO_ROLE_NAME=DifferentSyntheticRole\n"),
            format!("{SESSION}\n[sso-session\tdemo-session]\nsso_region=us-east-1\n"),
            format!(
                "{}\n{}",
                INLINE.replace("[profile DemoA]", "[default]"),
                INLINE.replace("[profile DemoA]", "[profile default]")
            ),
            SESSION.replace("[sso-session demo-session]", "[sso-session absent]"),
            INLINE.replace("sso_role_name = DemoReadOnly\n", ""),
        ] {
            let result = discover_text(&text, None);
            assert_eq!(result["profiles"].as_array().unwrap().len(), 1);
            assert_eq!(
                result["profiles"][0]["eligibility"], "invalid_sso",
                "{result}"
            );
        }
    }

    #[test]
    fn discovery_caps_profile_rows_and_does_not_echo_unbounded_metadata() {
        let text: String = (0..MAX_DISCOVERY_PROFILES + 3)
            .map(|number| INLINE.replace("DemoA]", &format!("Demo{number:04}]")))
            .collect();
        let result = discover_text(&text, None);
        assert_eq!(
            result["profiles"].as_array().unwrap().len(),
            MAX_DISCOVERY_PROFILES
        );
        assert_eq!(result["partial"], true);
        assert_eq!(
            result["coverage"],
            json!({"complete":false, "returned":MAX_DISCOVERY_PROFILES, "limit":MAX_DISCOVERY_PROFILES, "omitted":3})
        );
        let excessive = format!("synthetic-private-oversized-{}", "x".repeat(2048));
        let result = discover_text(
            &INLINE
                .replace("DemoReadOnly", &excessive)
                .replace("DemoA]", &format!("{excessive}]")),
            None,
        );
        assert_eq!(result["profiles"][0]["eligibility"], "invalid_sso");
        assert_eq!(result["profiles"][0]["name"], "(invalid profile name)");
        assert_eq!(result["profiles"][0]["role_name"], "");
        assert!(!result.to_string().contains("synthetic-private-oversized"));
    }

    // Construct unmistakably synthetic numeric IDs only where the production
    // parser requires AWS's numeric shape; source fixtures contain no account IDs.
    fn synthetic_account(number: u8) -> String {
        format!("{number:012}")
    }

    fn parse_snapshot(
        text: &str,
        selection: &SsoProfileSelection,
    ) -> Result<SsoProfileSnapshot, String> {
        SsoProfileSnapshot::parse(
            &text.replace("SYNTHETIC_ACCOUNT", &synthetic_account(1)),
            selection,
        )
    }

    const INLINE: &str = "[profile DemoA]\n\
        sso_start_url = https://example.awsapps.com/start\n\
        sso_region = us-east-1\n\
        sso_account_id = SYNTHETIC_ACCOUNT\n\
        sso_role_name = DemoReadOnly\n\
        region = us-east-1\n";

    const SESSION: &str = "[profile DemoA]\n\
        sso_session = demo-session\n\
        sso_account_id = SYNTHETIC_ACCOUNT\n\
        sso_role_name = DemoReadOnly\n\
        region = us-east-1\n\
        [sso-session demo-session]\n\
        sso_start_url = https://example.awsapps.com/start\n\
        sso_region = us-east-1\n\
        sso_registration_scopes = sso:account:access\n";

    fn selection() -> SsoProfileSelection {
        SsoProfileSelection {
            config_path: PathBuf::from("synthetic-config.ini"),
            profile: "DemoA".into(),
            expected_account_id: synthetic_account(1),
            requested_region: "eu-west-1".into(),
            session_override: None,
            settings_revision: 7,
        }
    }

    #[test]
    fn inline_sso_snapshot_captures_explicit_effective_selection() {
        let selected = selection();
        let snapshot = parse_snapshot(INLINE, &selected).unwrap();
        assert_eq!(snapshot.profile, "DemoA");
        assert_eq!(snapshot.account_id, synthetic_account(1));
        assert_eq!(snapshot.role_name, "DemoReadOnly");
        assert_eq!(snapshot.region, "eu-west-1");
        assert_eq!(snapshot.sso_region, "us-east-1");
        assert_eq!(snapshot.config_path, selected.config_path);
        assert_eq!(snapshot.settings_revision, 7);
        assert_eq!(snapshot.session_name, None);
        assert_eq!(snapshot.registration_scopes, None);
        assert_eq!(
            snapshot.token_cache_key(),
            "https://example.awsapps.com/start"
        );
    }

    #[test]
    fn session_sso_snapshot_resolves_only_its_referenced_session() {
        let mut selected = selection();
        selected.session_override = Some("demo-session".into());
        let snapshot = parse_snapshot(SESSION, &selected).unwrap();
        assert_eq!(snapshot.session_name.as_deref(), Some("demo-session"));
        assert_eq!(snapshot.token_cache_key(), "demo-session");
        assert_eq!(
            snapshot.registration_scopes.as_deref(),
            Some("sso:account:access")
        );
        let repeated = SESSION.replace(
            "sso_session = demo-session\n",
            "sso_session = demo-session\nsso_start_url = https://example.awsapps.com/start\nsso_region = us-east-1\n",
        );
        assert_eq!(parse_snapshot(&repeated, &selected).unwrap(), snapshot);
    }

    #[test]
    fn single_logical_profile_and_session_aliases_resolve_identically() {
        let expected = parse_snapshot(INLINE, &selection()).unwrap();
        for header in [
            "profile   DemoA",
            "profile\tDemoA",
            " \tprofile \t DemoA\t ",
        ] {
            let text = INLINE.replace("[profile DemoA]", &format!("[{header}]"));
            assert_eq!(parse_snapshot(&text, &selection()).unwrap(), expected);
        }
        let expected = parse_snapshot(SESSION, &selection()).unwrap();
        for header in [
            "sso-session   demo-session",
            "sso-session\tdemo-session",
            " \tsso-session \t demo-session\t ",
        ] {
            let text = SESSION.replace("[sso-session demo-session]", &format!("[{header}]"));
            assert_eq!(parse_snapshot(&text, &selection()).unwrap(), expected);
        }
    }

    #[test]
    fn each_default_spelling_resolves_but_combined_aliases_are_ambiguous() {
        let mut selected = selection();
        selected.profile = "default".into();
        let default = INLINE.replace("[profile DemoA]", "[default]");
        let expected = parse_snapshot(&default, &selected).unwrap();
        for header in ["profile default", "profile   default", "profile\tdefault"] {
            let alias = INLINE.replace("[profile DemoA]", &format!("[{header}]"));
            assert_eq!(parse_snapshot(&alias, &selected).unwrap(), expected);
            for combined in [format!("{default}\n{alias}"), format!("{alias}\n{default}")] {
                assert!(parse_snapshot(&combined, &selected)
                    .unwrap_err()
                    .contains("ambiguous duplicate sections"));
            }
        }
    }

    #[test]
    fn logical_alias_duplicates_cannot_hide_provider_or_endpoint_settings() {
        for header in ["profile DemoA", "profile   DemoA", "profile\tDemoA"] {
            let unsafe_alias =
                format!("[{header}]\ncredential_process = SYNTHETIC-NEVER-EXECUTED\n");
            for text in [
                format!("{INLINE}\n{unsafe_alias}"),
                format!("{unsafe_alias}\n{INLINE}"),
            ] {
                let error = parse_snapshot(&text, &selection()).unwrap_err();
                assert!(error.contains("ambiguous duplicate sections"));
                assert!(!error.contains("SYNTHETIC-NEVER-EXECUTED"));
            }
        }
        for header in [
            "sso-session demo-session",
            "sso-session   demo-session",
            "sso-session\tdemo-session",
        ] {
            let unsafe_alias = format!("[{header}]\nendpoint_url = https://example.invalid\n");
            for text in [
                format!("{SESSION}\n{unsafe_alias}"),
                format!("{unsafe_alias}\n{SESSION}"),
            ] {
                assert!(parse_snapshot(&text, &selection())
                    .unwrap_err()
                    .contains("ambiguous duplicate sections"));
            }
        }
    }

    #[test]
    fn account_session_and_inline_conflicts_fail_before_provider_selection() {
        let mut selected = selection();
        selected.expected_account_id = synthetic_account(2);
        assert!(parse_snapshot(INLINE, &selected)
            .unwrap_err()
            .contains("does not match"));
        selected = selection();
        selected.session_override = Some("another-demo-session".into());
        assert!(parse_snapshot(SESSION, &selected).is_err());
        assert!(parse_snapshot(INLINE, &selected).is_err());
        for inline in [
            "sso_region = eu-west-1",
            "sso_start_url = https://other.example.invalid/start",
            "sso_registration_scopes = conflicting:scope",
        ] {
            let conflicting = SESSION.replace(
                "sso_session = demo-session\n",
                &format!("sso_session = demo-session\n{inline}\n"),
            );
            assert!(parse_snapshot(&conflicting, &selection())
                .unwrap_err()
                .contains("conflicting"));
        }
    }

    #[test]
    fn missing_fields_or_referenced_session_do_not_fall_back() {
        for field in [
            "sso_start_url = https://example.awsapps.com/start\n",
            "sso_region = us-east-1\n",
            "sso_account_id = SYNTHETIC_ACCOUNT\n",
            "sso_role_name = DemoReadOnly\n",
        ] {
            assert!(parse_snapshot(&INLINE.replace(field, ""), &selection()).is_err());
        }
        let missing_session = SESSION.replace("[sso-session demo-session]", "[sso-session other]");
        assert!(parse_snapshot(&missing_session, &selection()).is_err());
        let incomplete_session = SESSION.replace("sso_region = us-east-1\n", "").replace(
            "sso_session = demo-session\n",
            "sso_session = demo-session\nsso_region = us-east-1\n",
        );
        assert!(parse_snapshot(&incomplete_session, &selection()).is_err());
        let default_only = INLINE.replace("[profile DemoA]", "[default]");
        assert!(parse_snapshot(&default_only, &selection()).is_err());
        let mut selected = selection();
        selected.profile = "default".into();
        assert!(parse_snapshot(&default_only, &selected).is_ok());
    }

    #[test]
    fn unsupported_credential_and_endpoint_settings_are_rejected() {
        for key in [
            "credential_process",
            "aws_access_key_id",
            "aws_secret_access_key",
            "aws_session_token",
            "aws_security_token",
            "role_arn",
            "source_profile",
            "credential_source",
            "web_identity_token_file",
            "role_session_name",
            "external_id",
            "mfa_serial",
            "endpoint_url",
            "services",
            "ca_bundle",
            "use_fips_endpoint",
            "use_dualstack_endpoint",
            "AWS_PROFILE",
        ] {
            for input in [INLINE, SESSION] {
                let text = format!("{input}{key} = SYNTHETIC-NOT-A-CREDENTIAL\n");
                let error = parse_snapshot(&text, &selection()).unwrap_err();
                assert!(error.contains("unsupported setting"), "key {key} must fail");
                assert!(!error.contains("SYNTHETIC-NOT-A-CREDENTIAL"));
            }
        }
    }

    #[test]
    fn duplicates_malformed_config_and_invalid_values_fail_closed() {
        for text in [
            format!("{INLINE}\n{INLINE}"),
            format!("{INLINE}sso_role_name = AnotherDemoRole\n"),
            format!("{INLINE}SSO_ROLE_NAME = DemoReadOnly\n"),
            format!("{SESSION}\n[sso-session demo-session]\nsso_region = us-east-1\n"),
            "[profile DemoA\nsso_role_name = SYNTHETIC-SENSITIVE-TEXT".into(),
            INLINE.replace("sso_role_name = DemoReadOnly", "sso_role_name ="),
            INLINE.replace("SYNTHETIC_ACCOUNT", "123"),
            INLINE.replace(
                "sso_region = us-east-1",
                "sso_region = https://example.invalid",
            ),
            INLINE.replace(
                "https://example.awsapps.com/start",
                "http://example.invalid/start",
            ),
            INLINE.replace(
                "https://example.awsapps.com/start",
                "https://user@example.invalid/start",
            ),
        ] {
            let error = parse_snapshot(&text, &selection()).unwrap_err();
            assert!(!error.contains("SYNTHETIC-SENSITIVE-TEXT"));
        }
        let mut selected = selection();
        selected.profile = "DemoA\n".into();
        assert!(parse_snapshot(INLINE, &selected).is_err());
        selected = selection();
        selected.requested_region = "".into();
        assert!(parse_snapshot(INLINE, &selected).is_err());
        selected = selection();
        selected.config_path = PathBuf::new();
        assert!(parse_snapshot(INLINE, &selected).is_err());
    }

    #[test]
    fn unrelated_profiles_and_inert_preferences_never_enter_the_snapshot() {
        let text = format!(
            "{INLINE}output = json\nretry_mode = standard\ncli_pager = synthetic-pager\n\
             [profile Other]\ncredential_process = SYNTHETIC-NEVER-EXECUTED\n\
             [default]\naws_secret_access_key = SYNTHETIC-NEVER-LOADED\n"
        );
        let snapshot = parse_snapshot(&text, &selection()).unwrap();
        assert_eq!(snapshot, parse_snapshot(INLINE, &selection()).unwrap());
        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("SYNTHETIC-NEVER"));
        assert!(!debug.contains("synthetic-pager"));
    }

    #[test]
    fn snapshot_equality_tracks_identity_settings_and_config_changes() {
        let original = parse_snapshot(INLINE, &selection()).unwrap();
        assert_eq!(original.clone(), original);
        let changed_config = INLINE.replace("DemoReadOnly", "AnotherDemoRole");
        assert_ne!(
            original,
            parse_snapshot(&changed_config, &selection()).unwrap()
        );
        let mut selected = selection();
        selected.settings_revision += 1;
        assert_ne!(original, parse_snapshot(INLINE, &selected).unwrap());
        selected = selection();
        selected.config_path = PathBuf::from("another-synthetic-config.ini");
        assert_ne!(original, parse_snapshot(INLINE, &selected).unwrap());
        selected = selection();
        selected.requested_region = "us-east-1".into();
        assert_ne!(original, parse_snapshot(INLINE, &selected).unwrap());
    }
}
