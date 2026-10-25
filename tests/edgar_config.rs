//! EdgarConfig contract: defaults, User-Agent validation, env-var override.
//!
//! Required API:
//!   EdgarConfig { pub user_agent: String, pub requests_per_second: u32, pub cache_dir: PathBuf,
//!                 pub cache_ttl_hours: u64, pub max_retries: u32, pub retry_base_delay_ms: u64 }
//!   EdgarConfig::new(user_agent: impl Into<String>, cache_dir: impl Into<PathBuf>) -> EdgarConfig
//!       defaults: requests_per_second 8, cache_ttl_hours 24, max_retries 3, retry_base_delay_ms 500
//!       (new() does NOT validate)
//!   EdgarConfig::validate(&self) -> Result<(), EdgarError>
//!   EdgarConfig::resolve(config_user_agent: Option<&str>, env_user_agent: Option<&str>,
//!                        cache_dir: PathBuf) -> Result<EdgarConfig, EdgarError>
//!       env value (when Some and non-blank) wins over the config value; neither => InvalidUserAgent;
//!       the winning value is validated.
//!   The CLI reads env var GHOST_ALPHA_EDGAR_USER_AGENT (covered in cli_edgar.rs).
//!   EdgarError::{InvalidUserAgent(String), InvalidConfig(String)}

use ghost_alpha::data::edgar::*;
use std::path::PathBuf;

const GOOD_UA: &str = "Example Research jane.doe@example.com";

fn cfg(ua: &str) -> EdgarConfig {
    EdgarConfig::new(ua, PathBuf::from("/tmp/ghost_alpha_edgar_unused"))
}

#[test]
fn test_config_new_when_defaults_should_match_plan() {
    let c = cfg(GOOD_UA);
    assert_eq!(c.user_agent, GOOD_UA);
    assert_eq!(c.requests_per_second, 8);
    assert_eq!(c.cache_ttl_hours, 24);
    assert_eq!(c.max_retries, 3);
    assert_eq!(c.retry_base_delay_ms, 500);
    assert_eq!(c.cache_dir, PathBuf::from("/tmp/ghost_alpha_edgar_unused"));
}

#[test]
fn test_validate_when_user_agent_has_email_should_be_ok() {
    assert!(cfg(GOOD_UA).validate().is_ok());
    assert!(cfg("jane@example.com").validate().is_ok());
}

#[test]
fn test_validate_when_user_agent_empty_or_blank_should_error_invalid_user_agent() {
    for ua in ["", "   ", "\t"] {
        assert!(
            matches!(cfg(ua).validate(), Err(EdgarError::InvalidUserAgent(_))),
            "{ua:?}"
        );
    }
}

#[test]
fn test_validate_when_user_agent_has_no_email_token_should_error_invalid_user_agent() {
    for ua in [
        "MyApp/1.0",
        "Example Research",
        "me@",
        "@example.com",
        "me @ example.com",
    ] {
        assert!(
            matches!(cfg(ua).validate(), Err(EdgarError::InvalidUserAgent(_))),
            "{ua:?}"
        );
    }
}

#[test]
fn test_validate_when_zero_requests_per_second_should_error_invalid_config() {
    let mut c = cfg(GOOD_UA);
    c.requests_per_second = 0;
    assert!(matches!(c.validate(), Err(EdgarError::InvalidConfig(_))));
}

#[test]
fn test_resolve_when_only_config_value_should_use_it() {
    let c = EdgarConfig::resolve(Some(GOOD_UA), None, PathBuf::from("/x")).unwrap();
    assert_eq!(c.user_agent, GOOD_UA);
    assert_eq!(c.cache_dir, PathBuf::from("/x"));
    assert_eq!(c.requests_per_second, 8);
}

#[test]
fn test_resolve_when_env_present_should_override_config_value() {
    let c = EdgarConfig::resolve(
        Some("Config Bot cfg@example.com"),
        Some("Env Bot env@example.com"),
        PathBuf::from("/x"),
    )
    .unwrap();
    assert_eq!(c.user_agent, "Env Bot env@example.com");
}

#[test]
fn test_resolve_when_env_only_should_use_env() {
    let c =
        EdgarConfig::resolve(None, Some("Env Bot env@example.com"), PathBuf::from("/x")).unwrap();
    assert_eq!(c.user_agent, "Env Bot env@example.com");
}

#[test]
fn test_resolve_when_env_blank_should_fall_back_to_config() {
    let c = EdgarConfig::resolve(Some(GOOD_UA), Some("  "), PathBuf::from("/x")).unwrap();
    assert_eq!(c.user_agent, GOOD_UA);
}

#[test]
fn test_resolve_when_nothing_configured_should_error_invalid_user_agent() {
    for (cfg_ua, env_ua) in [(None, None), (Some(""), None), (Some(""), Some(""))] {
        let r = EdgarConfig::resolve(cfg_ua, env_ua, PathBuf::from("/x"));
        assert!(
            matches!(r, Err(EdgarError::InvalidUserAgent(_))),
            "{cfg_ua:?} {env_ua:?}"
        );
    }
}

#[test]
fn test_resolve_when_winning_value_has_no_email_should_error_even_if_other_is_valid() {
    let r = EdgarConfig::resolve(Some(GOOD_UA), Some("no-email-here"), PathBuf::from("/x"));
    assert!(matches!(r, Err(EdgarError::InvalidUserAgent(_))));
}

#[test]
fn test_client_new_when_user_agent_invalid_should_error_invalid_user_agent() {
    let r = EdgarClient::new(cfg("nope"));
    assert!(matches!(r, Err(EdgarError::InvalidUserAgent(_))));
}

#[test]
fn test_client_new_when_config_valid_should_succeed() {
    assert!(EdgarClient::new(cfg(GOOD_UA)).is_ok());
}
