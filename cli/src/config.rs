//! `atlas.toml` configuration parser.
//!
//! A developer drops `atlas.toml` in their project root to configure which
//! Atlas platform services they want. This module parses and validates that
//! file. Unknown keys are hard errors (typo protection). Missing required
//! fields are hard errors. Cross-section consistency is checked after the
//! deserializer runs.
//!
//! Topic names in `[events].topics` are the *developer-facing* short form
//! (e.g. `"location.updates"`). The platform prepends `atlas.` internally
//! to produce the actual Kafka topic name. See `KNOWN_TOPICS`.

use serde::Deserialize;
use std::fs;
use std::path::Path;
use thiserror::Error;

/// Topic short-names a developer is allowed to enable in `atlas.toml`.
/// The platform internally maps these to fully-qualified `atlas.*` topics.
pub const KNOWN_TOPICS: &[&str] = &[
    "location.updates",
    "fare.events",
    "auth.tokens",
    "safety.alerts",
    "elo.recompute",
];

/// Regions where Atlas projects can be provisioned.
pub const KNOWN_REGIONS: &[&str] = &["us-central1", "eu-west1", "ap-southeast1"];

/// API key prefixes. Each maps to a deployment tier.
const KEY_PREFIXES: &[&str] = &["atl_live_", "atl_test_", "atl_dev_"];

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid TOML in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("validation failed: {0}")]
    Validation(String),
}

/// Top-level config. Every section uses `deny_unknown_fields` so typos
/// like `[paymetns]` surface as an error instead of silently being ignored.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AtlasConfig {
    pub project: Project,
    pub services: Services,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub geo: Option<GeoConfig>,
    #[serde(default)]
    pub payments: Option<PaymentsConfig>,
    #[serde(default)]
    pub events: Option<EventsConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub api_key: String,
    pub environment: Environment,
    pub region: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Development => "development",
            Environment::Staging => "staging",
            Environment::Production => "production",
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Services {
    #[serde(default)]
    pub auth: bool,
    #[serde(default)]
    pub geo: bool,
    #[serde(default)]
    pub payments: bool,
    #[serde(default)]
    pub events: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default = "default_token_expiry")]
    pub token_expiry_seconds: u32,
    #[serde(default = "default_true")]
    pub geo_claims: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct GeoConfig {
    #[serde(default = "default_radius")]
    pub default_radius_m: u32,
    #[serde(default = "default_elo_cron")]
    pub elo_recompute_cron: String,
    #[serde(default = "default_location_ttl")]
    pub location_ttl_hours: u32,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct PaymentsConfig {
    #[serde(default = "default_currency")]
    pub currency: String,
    #[serde(default)]
    pub settlement_mode: SettlementMode,
    #[serde(default = "default_idempotency_window")]
    pub idempotency_window_s: u32,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SettlementMode {
    #[default]
    Auto,
    Manual,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EventsConfig {
    pub topics: Vec<String>,
    #[serde(default = "default_retention_hours")]
    pub retention_hours: u32,
}

// --- Defaults --------------------------------------------------------------

fn default_token_expiry() -> u32 {
    3600
}
fn default_true() -> bool {
    true
}
fn default_radius() -> u32 {
    500
}
fn default_elo_cron() -> String {
    "0 3 * * *".to_string()
}
fn default_location_ttl() -> u32 {
    24
}
fn default_currency() -> String {
    "USD".to_string()
}
fn default_idempotency_window() -> u32 {
    86_400
}
fn default_retention_hours() -> u32 {
    168
}

// --- Loading + validation --------------------------------------------------

impl AtlasConfig {
    /// Read and validate `atlas.toml` from a path. Validation includes both
    /// `serde`'s structural checks and the semantic checks in `validate()`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path_str = path.as_ref().display().to_string();
        let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Io {
            path: path_str.clone(),
            source,
        })?;
        Self::parse_str(&raw, &path_str)
    }

    /// Parse a config from a string. Public for unit tests.
    pub fn parse_str(raw: &str, path_label: &str) -> Result<Self, ConfigError> {
        let cfg: AtlasConfig = toml::from_str(raw).map_err(|source| ConfigError::Parse {
            path: path_label.to_string(),
            source,
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Semantic validation that can't be expressed in the serde schema.
    fn validate(&self) -> Result<(), ConfigError> {
        // --- project ---
        if self.project.name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "project.name must not be empty".into(),
            ));
        }
        if !is_valid_project_name(&self.project.name) {
            return Err(ConfigError::Validation(format!(
                "project.name '{}' is invalid: must be 3-40 chars, lowercase alphanumeric and hyphens, \
                 and not start or end with a hyphen",
                self.project.name
            )));
        }
        if !KEY_PREFIXES.iter().any(|p| self.project.api_key.starts_with(p)) {
            return Err(ConfigError::Validation(format!(
                "project.api_key must start with one of {:?}",
                KEY_PREFIXES
            )));
        }
        if self.project.api_key.len() < 24 {
            return Err(ConfigError::Validation(
                "project.api_key looks too short to be valid".into(),
            ));
        }
        if !KNOWN_REGIONS.contains(&self.project.region.as_str()) {
            return Err(ConfigError::Validation(format!(
                "project.region '{}' is not a known Atlas region. Known: {:?}",
                self.project.region, KNOWN_REGIONS
            )));
        }

        // --- services consistency ---
        if !(self.services.auth
            || self.services.geo
            || self.services.payments
            || self.services.events)
        {
            return Err(ConfigError::Validation(
                "[services] must enable at least one of auth, geo, payments, events".into(),
            ));
        }

        // --- per-service sections only meaningful when the service is enabled ---
        // We do NOT error if e.g. [auth] is present but services.auth = false —
        // that's a soft mistake we'll warn about during `validate` command output.
        // We DO error on invalid values inside enabled sections.
        if self.services.auth {
            if let Some(a) = &self.auth {
                if a.token_expiry_seconds == 0 {
                    return Err(ConfigError::Validation(
                        "auth.token_expiry_seconds must be > 0".into(),
                    ));
                }
                if a.token_expiry_seconds > 60 * 60 * 24 * 30 {
                    return Err(ConfigError::Validation(
                        "auth.token_expiry_seconds must be <= 30 days (2592000)".into(),
                    ));
                }
            }
        }

        if self.services.geo {
            if let Some(g) = &self.geo {
                if g.default_radius_m == 0 {
                    return Err(ConfigError::Validation(
                        "geo.default_radius_m must be > 0".into(),
                    ));
                }
                if g.default_radius_m > 50_000 {
                    return Err(ConfigError::Validation(
                        "geo.default_radius_m must be <= 50000 (50km)".into(),
                    ));
                }
                if g.location_ttl_hours == 0 {
                    return Err(ConfigError::Validation(
                        "geo.location_ttl_hours must be > 0".into(),
                    ));
                }
                if !is_plausible_cron(&g.elo_recompute_cron) {
                    return Err(ConfigError::Validation(format!(
                        "geo.elo_recompute_cron '{}' does not look like a 5-field cron expression",
                        g.elo_recompute_cron
                    )));
                }
            }
        }

        if self.services.payments {
            if let Some(p) = &self.payments {
                if p.currency.len() != 3 || !p.currency.chars().all(|c| c.is_ascii_uppercase()) {
                    return Err(ConfigError::Validation(format!(
                        "payments.currency '{}' must be a 3-letter ISO 4217 code (e.g. USD)",
                        p.currency
                    )));
                }
                if p.idempotency_window_s == 0 {
                    return Err(ConfigError::Validation(
                        "payments.idempotency_window_s must be > 0".into(),
                    ));
                }
            }
        }

        if self.services.events {
            let events = self.events.as_ref().ok_or_else(|| {
                ConfigError::Validation(
                    "[events] section is required when services.events = true".into(),
                )
            })?;
            if events.topics.is_empty() {
                return Err(ConfigError::Validation(
                    "events.topics must list at least one topic".into(),
                ));
            }
            for t in &events.topics {
                if !KNOWN_TOPICS.contains(&t.as_str()) {
                    return Err(ConfigError::Validation(format!(
                        "events.topics contains unknown topic '{}'. Known topics: {:?}",
                        t, KNOWN_TOPICS
                    )));
                }
            }
            if events.retention_hours == 0 || events.retention_hours > 720 {
                return Err(ConfigError::Validation(
                    "events.retention_hours must be between 1 and 720 (30 days)".into(),
                ));
            }
        }

        Ok(())
    }

    /// Sections that are present in the file but whose service is disabled.
    /// Returned as warnings (non-fatal) by the `validate` command.
    pub fn dangling_sections(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.auth.is_some() && !self.services.auth {
            out.push("auth");
        }
        if self.geo.is_some() && !self.services.geo {
            out.push("geo");
        }
        if self.payments.is_some() && !self.services.payments {
            out.push("payments");
        }
        if self.events.is_some() && !self.services.events {
            out.push("events");
        }
        out
    }

    /// Mask the API key for display. Keeps the prefix and last 4 chars.
    pub fn masked_api_key(&self) -> String {
        mask_key(&self.project.api_key)
    }
}

pub fn mask_key(key: &str) -> String {
    // Find the prefix portion (everything up to and including the last `_`)
    let prefix_end = key.rfind('_').map(|i| i + 1).unwrap_or(0);
    let (prefix, secret) = key.split_at(prefix_end);
    if secret.len() <= 4 {
        return format!("{}{}", prefix, "*".repeat(secret.len()));
    }
    let tail = &secret[secret.len() - 4..];
    format!("{}{}{}", prefix, "*".repeat(secret.len() - 4), tail)
}

fn is_valid_project_name(name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    let len = chars.len();
    if !(3..=40).contains(&len) {
        return false;
    }
    if chars[0] == '-' || chars[len - 1] == '-' {
        return false;
    }
    chars
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
}

/// Quick sanity check for a cron expression: 5 whitespace-separated fields.
/// We don't fully parse cron — the control plane does that authoritatively.
fn is_plausible_cron(s: &str) -> bool {
    s.split_whitespace().count() == 5
}

// --- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_VALID: &str = r#"
[project]
name        = "my-mobility-app"
api_key     = "atl_live_abcdef0123456789abcdef0123456789"
environment = "production"
region      = "us-central1"

[services]
auth     = true
geo      = true
payments = true
events   = true

[auth]
token_expiry_seconds = 3600
geo_claims           = true

[geo]
default_radius_m     = 500
elo_recompute_cron   = "0 3 * * *"
location_ttl_hours   = 24

[payments]
currency             = "USD"
settlement_mode      = "auto"
idempotency_window_s = 86400

[events]
topics = ["location.updates", "fare.events", "auth.tokens", "safety.alerts"]
retention_hours = 168
"#;

    fn parse(toml_str: &str) -> Result<AtlasConfig, ConfigError> {
        AtlasConfig::parse_str(toml_str, "<test>")
    }

    #[test]
    fn full_valid_config_parses() {
        let cfg = parse(FULL_VALID).expect("should parse");
        assert_eq!(cfg.project.name, "my-mobility-app");
        assert_eq!(cfg.project.environment, Environment::Production);
        assert!(cfg.services.auth);
        assert_eq!(cfg.events.unwrap().topics.len(), 4);
    }

    #[test]
    fn minimal_valid_config() {
        let cfg = parse(
            r#"
[project]
name        = "tiny"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
auth = true
"#,
        )
        .expect("should parse minimal config");
        assert!(cfg.services.auth);
        assert!(!cfg.services.geo);
    }

    #[test]
    fn unknown_top_level_key_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
auth = true

[oops]
foo = "bar"
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "got {err:?}");
    }

    #[test]
    fn unknown_field_in_section_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"
typo_field  = "boom"

[services]
auth = true
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "got {err:?}");
    }

    #[test]
    fn missing_required_project_field_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
environment = "development"
region      = "us-central1"

[services]
auth = true
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn invalid_environment_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "prod"
region      = "us-central1"

[services]
auth = true
"#,
        )
        .unwrap_err();
        // serde rejects the enum variant
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn bad_api_key_prefix_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "wrongprefix_abcdefghijklmnopqrstuvwx"
environment = "development"
region      = "us-central1"

[services]
auth = true
"#,
        )
        .unwrap_err();
        match err {
            ConfigError::Validation(msg) => assert!(msg.contains("api_key")),
            _ => panic!("expected Validation, got {err:?}"),
        }
    }

    #[test]
    fn short_api_key_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_short"
environment = "development"
region      = "us-central1"

[services]
auth = true
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn unknown_region_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "mars-1"

[services]
auth = true
"#,
        )
        .unwrap_err();
        match err {
            ConfigError::Validation(msg) => assert!(msg.contains("region")),
            _ => panic!("expected Validation"),
        }
    }

    #[test]
    fn no_services_enabled_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
"#,
        )
        .unwrap_err();
        match err {
            ConfigError::Validation(msg) => assert!(msg.contains("at least one")),
            _ => panic!("expected Validation"),
        }
    }

    #[test]
    fn invalid_project_name_is_rejected() {
        for bad in ["-leading", "trailing-", "UPPER", "with space", "ok!", "ab"] {
            let toml = format!(
                r#"
[project]
name        = "{bad}"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
auth = true
"#
            );
            let err = parse(&toml).unwrap_err();
            assert!(matches!(err, ConfigError::Validation(_)), "name {bad:?} should be rejected, got {err:?}");
        }
    }

    #[test]
    fn unknown_topic_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
events = true

[events]
topics = ["location.updates", "made.up.topic"]
"#,
        )
        .unwrap_err();
        match err {
            ConfigError::Validation(msg) => assert!(msg.contains("made.up.topic")),
            _ => panic!("expected Validation"),
        }
    }

    #[test]
    fn empty_topics_list_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
events = true

[events]
topics = []
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn events_enabled_without_section_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
events = true
"#,
        )
        .unwrap_err();
        match err {
            ConfigError::Validation(msg) => assert!(msg.contains("[events] section is required")),
            _ => panic!("expected Validation"),
        }
    }

    #[test]
    fn bad_currency_is_rejected() {
        for bad in ["usd", "DOLLARS", "U$D"] {
            let toml = format!(
                r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
payments = true

[payments]
currency = "{bad}"
"#
            );
            let err = parse(&toml).unwrap_err();
            assert!(matches!(err, ConfigError::Validation(_)), "currency {bad:?} should be rejected, got {err:?}");
        }
    }

    #[test]
    fn implausible_cron_is_rejected() {
        let err = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
geo = true

[geo]
elo_recompute_cron = "every night"
"#,
        )
        .unwrap_err();
        match err {
            ConfigError::Validation(msg) => assert!(msg.contains("cron")),
            _ => panic!("expected Validation"),
        }
    }

    #[test]
    fn token_expiry_bounds() {
        // zero
        let zero = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
auth = true

[auth]
token_expiry_seconds = 0
"#,
        );
        assert!(matches!(zero, Err(ConfigError::Validation(_))));

        // too big
        let huge = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
auth = true

[auth]
token_expiry_seconds = 999999999
"#,
        );
        assert!(matches!(huge, Err(ConfigError::Validation(_))));
    }

    #[test]
    fn dangling_section_is_warning_not_error() {
        let cfg = parse(
            r#"
[project]
name        = "app"
api_key     = "atl_dev_xxxxxxxxxxxxxxxxxxxxxxxx"
environment = "development"
region      = "us-central1"

[services]
auth = true

[geo]
default_radius_m = 500
"#,
        )
        .expect("should parse — dangling [geo] is not fatal");
        assert_eq!(cfg.dangling_sections(), vec!["geo"]);
    }

    #[test]
    fn key_masking() {
        assert_eq!(
            mask_key("atl_live_abcdef0123456789abcdef"),
            "atl_live_******************cdef"
        );
        assert_eq!(mask_key("atl_dev_abc"), "atl_dev_***");
    }
}
