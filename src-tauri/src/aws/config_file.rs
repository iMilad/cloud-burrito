//! Read SSO-relevant profile metadata from an AWS config file (`~/.aws/config`).
//!
//! Powers account/profile discovery and pure validation of the selected SSO
//! configuration. Verified credential expiry is reported by the context layer.

use std::collections::BTreeMap;
use std::path::PathBuf;

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
        if selection.config_path.as_os_str().is_empty() {
            return Err("an explicit AWS configuration path is required".into());
        }
        validate_name(&selection.profile, "selected profile")?;
        validate_account(&selection.expected_account_id)?;
        validate_region(&selection.requested_region, "selected region")?;
        let ini = Ini::load_from_str_noescape(text)
            .map_err(|_| "selected AWS configuration is not valid INI".to_string())?;
        let profile = selected_properties(&ini, &selection.profile, "selected SSO profile", false)?;
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
                &ini,
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
    ini: &Ini,
    name: &str,
    description: &str,
    session: bool,
) -> Result<ConfigProperties, String> {
    let mut sections = ini.iter().filter_map(|(section, properties)| {
        (logical_section_name(section?, session) == Some(name)).then_some(properties)
    });
    let properties = sections
        .next()
        .ok_or_else(|| format!("{description} is missing from the selected configuration"))?;
    if sections.next().is_some() {
        return Err(format!("{description} has ambiguous duplicate sections"));
    }
    let mut selected = BTreeMap::new();
    for (key, value) in properties.iter() {
        let key = key.trim().to_ascii_lowercase();
        let supported = matches!(
            key.as_str(),
            "sso_start_url" | "sso_region" | "sso_registration_scopes"
        ) || (!session
            && matches!(
                key.as_str(),
                "sso_session" | "sso_account_id" | "sso_role_name" | "region"
                    // Inert preferences are validated but are never forwarded to
                    // a provider, CLI child, endpoint resolver, or config loader.
                    | "output" | "cli_pager" | "cli_auto_prompt" | "cli_history"
                    | "cli_timestamp_format" | "cli_binary_format" | "retry_mode"
                    | "max_attempts" | "parameter_validation" | "tcp_keepalive"
            ));
        if !supported {
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

#[cfg(test)]
mod tests {
    use super::*;

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
