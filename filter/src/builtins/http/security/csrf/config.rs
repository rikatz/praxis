// SPDX-License-Identifier: MIT
// Copyright (c) 2024 Shane Utt

//! Deserialized YAML configuration types for the CSRF filter.

use serde::{Deserialize, Serialize};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

/// Methods considered safe (no side effects) per [RFC 9110 Section 9.2.1].
///
/// [RFC 9110 Section 9.2.1]: https://datatracker.ietf.org/doc/html/rfc9110#section-9.2.1
const DEFAULT_SAFE_METHODS: &[&str] = &["GET", "HEAD", "OPTIONS"];

// -----------------------------------------------------------------------------
// CsrfConfig
// -----------------------------------------------------------------------------

/// Deserialized YAML config for the CSRF filter.
///
/// ```yaml
/// filter: csrf
/// trusted_origins:
///   - "https://app.example.com"
///   - "https://*.example.com"
/// enforce_percentage: 100
/// enable_sec_fetch_site: true
/// ```
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CsrfConfig {
    /// Whether to also validate the `Sec-Fetch-Site` header.
    #[serde(default)]
    pub enable_sec_fetch_site: bool,

    /// Percentage of requests to enforce (0..=100).
    #[serde(default = "default_enforce_percentage")]
    pub enforce_percentage: u8,

    /// HTTP methods that bypass CSRF checks.
    #[serde(default = "default_safe_methods")]
    pub safe_methods: Vec<String>,

    /// Allowed origin values (scheme + host + optional port).
    pub trusted_origins: Vec<String>,
}

/// Default enforcement: 100% of requests.
fn default_enforce_percentage() -> u8 {
    100
}

/// Default safe methods per [RFC 9110].
///
/// [RFC 9110]: https://datatracker.ietf.org/doc/html/rfc9110
fn default_safe_methods() -> Vec<String> {
    DEFAULT_SAFE_METHODS.iter().map(|s| (*s).to_owned()).collect()
}

// -----------------------------------------------------------------------------
// Config Validation
// -----------------------------------------------------------------------------

/// Validate CSRF config rules at parse time.
///
/// # Errors
///
/// Returns an error if `trusted_origins` is empty or
/// `enforce_percentage` exceeds 100.
pub(super) fn validate_config(cfg: &CsrfConfig) -> Result<(), crate::FilterError> {
    if cfg.trusted_origins.is_empty() {
        return Err("csrf: trusted_origins must not be empty".into());
    }

    if cfg.enforce_percentage > 100 {
        let pct = cfg.enforce_percentage;
        return Err(format!("csrf: enforce_percentage must be 0..=100, got {pct}").into());
    }

    validate_origins(&cfg.trusted_origins)?;
    validate_safe_methods(&cfg.safe_methods)
}

/// Validate that each origin contains a scheme separator.
fn validate_origins(origins: &[String]) -> Result<(), crate::FilterError> {
    for origin in origins {
        if origin == "*" {
            continue;
        }
        if !origin.contains("://") {
            return Err(format!("csrf: origin \"{origin}\" must include scheme (e.g. https://example.com)").into());
        }
        if let Some(host) = origin.split_once("://").map(|(_, h)| h)
            && host.contains('*')
            && !host.starts_with("*.")
        {
            return Err(format!(
                "csrf: wildcard in origin \"{origin}\" must be at the start of the host (e.g. https://*.example.com)"
            )
            .into());
        }
    }

    Ok(())
}

/// Validate that `safe_methods` is not empty.
fn validate_safe_methods(methods: &[String]) -> Result<(), crate::FilterError> {
    if methods.is_empty() {
        return Err("csrf: safe_methods must not be empty".into());
    }

    Ok(())
}
