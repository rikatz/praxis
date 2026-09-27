// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Deserialized configuration types for `system_one_decision`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

/// Maximum body size allowed to be sent to the filter.
pub(super) const MAX_BODY_BYTES: usize = 32_768; // 32 KiB

/// Maximum configurable request bytes
pub(super) const MAX_REQUEST_BYTES: usize = 131_072; // 128 KiB
/// Maximum configurable response bytes
pub(super) const MAX_RESPONSE_BYTES: usize = 65_536; // 64 KiB
/// Maximum configurable timeout in milliseconds
pub(super) const MAX_TIMEOUT_MS: u64 = 10_000;

// -----------------------------------------------------------------------------
// SystemOneDecisionConfig
// -----------------------------------------------------------------------------

/// MVP configuration for one request-side System One evaluation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SystemOneDecisionConfig {
    /// Fixed System One endpoint URL.
    pub(super) endpoint: String,
    /// Wire model name sent in the request envelope.
    pub(super) model: String,
    /// Environment variable containing the optional bearer token.
    #[serde(default)]
    pub(super) api_key_env: Option<String>,
    /// Output state keys mapped to static strings or Praxis request sources.
    pub(super) state: BTreeMap<String, StateValueConfig>,
    /// Static questions sent together in each evaluation request.
    pub(super) questions: BTreeMap<String, SystemOneQuestionConfig>,
    /// Maximum serialized request size in bytes (1 through 131072).
    #[serde(default = "default_request_limit_size")]
    pub(super) request_size_limit_bytes: usize,
    /// Maximum serialized response size in bytes (1 through 65536).
    #[serde(default = "default_response_limit_size")]
    pub(super) response_size_limit_bytes: usize,
    /// Maximum outbound delivery time in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub(super) request_timeout_ms: u64,
}

/// One configured output-state value.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(super) enum StateValueConfig {
    /// Static value; arbitrary strings are sent unchanged.
    Static(String),
    /// Value sourced from the request context.
    Source(StateSourceConfig),
}

/// A named Praxis context source.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StateSourceConfig {
    /// One of request.method, request.path, request.query, request.headers,
    /// request.body, or client.ip.
    pub(super) from: String,
}

/// One of the supported System One question types.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub(super) enum SystemOneQuestionConfig {
    /// Probability of a configured yes proposition.
    Noul {
        /// Non-empty question instructions.
        instructions: String,
        /// Optional criteria for positive and negative answers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    /// Select one configured option.
    Choice {
        /// Non-empty question instructions.
        instructions: String,
        /// Configured options and optional text descriptions.
        criteria: BTreeMap<String, Option<String>>,
    },
    /// Score against an ordered rubric.
    Score {
        /// Non-empty question instructions.
        instructions: String,
        /// Ordered rubric levels.
        criteria: Vec<String>,
    },
}

impl SystemOneDecisionConfig {
    /// Validate values that can be checked without a request or backend call.
    pub(super) fn validate(&self) -> Result<(), String> {
        let endpoint: http::Uri = self
            .endpoint
            .parse()
            .map_err(|error| format!("'endpoint' is not a valid absolute HTTP URL: {error}"))?;

        if !matches!(endpoint.scheme_str(), Some("http" | "https")) || endpoint.host().is_none() {
            return Err("'endpoint' must be an absolute http:// or https:// URL with a host".into());
        }

        if self.model.trim().is_empty() {
            return Err("'model' must not be empty".into());
        }

        if self.state.is_empty() {
            return Err("'state' must contain at least one mapping".into());
        }

        if self.request_size_limit_bytes == 0 || self.request_size_limit_bytes > MAX_REQUEST_BYTES {
            return Err(format!(
                "'request_size_limit_bytes' must be in 1..={MAX_REQUEST_BYTES}, got {}",
                self.request_size_limit_bytes,
            ));
        }

        if self.response_size_limit_bytes == 0 || self.response_size_limit_bytes > MAX_RESPONSE_BYTES {
            return Err(format!(
                "'response_size_limit_bytes' must be in 1..={MAX_RESPONSE_BYTES}, got {}",
                self.response_size_limit_bytes,
            ));
        }

        if self.request_timeout_ms == 0 || self.request_timeout_ms > MAX_TIMEOUT_MS {
            return Err(format!(
                "'request_timeout_ms' must be in 1..={MAX_TIMEOUT_MS}, got {}",
                self.request_timeout_ms,
            ));
        }

        for (key, value) in &self.state {
            if key.trim().is_empty() {
                return Err("state output keys must not be empty".into());
            }

            if let StateValueConfig::Source(source) = value
                && !matches!(
                    source.from.as_str(),
                    "request.method"
                        | "request.path"
                        | "request.query"
                        | "request.headers"
                        | "request.body"
                        | "client.ip"
                )
            {
                return Err(format!("unsupported state source '{}'", source.from));
            }
        }

        if self.questions.is_empty() {
            return Err("'questions' must not be empty".into());
        }

        for (id, question) in &self.questions {
            if id.trim().is_empty() {
                return Err("question IDs must not be empty".into());
            }

            let instructions = match question {
                SystemOneQuestionConfig::Noul { instructions, criteria } => {
                    if let Some(criteria) = criteria {
                        if criteria.positive.is_none() && criteria.negative.is_none() {
                            return Err(format!("question '{id}' has empty noul criteria"));
                        }
                        if criteria
                            .positive
                            .as_deref()
                            .is_some_and(|value| value.trim().is_empty())
                            || criteria
                                .negative
                                .as_deref()
                                .is_some_and(|value| value.trim().is_empty())
                        {
                            return Err(format!("question '{id}' has a blank noul criterion"));
                        }
                    }
                    instructions
                },
                SystemOneQuestionConfig::Choice { instructions, criteria } => {
                    if criteria.is_empty() {
                        return Err(format!("question '{id}' must have at least one choice option"));
                    }
                    if criteria.keys().any(|label| label.trim().is_empty()) {
                        return Err(format!("question '{id}' has an empty choice label"));
                    }
                    if criteria
                        .values()
                        .any(|description| description.as_deref().is_some_and(|value| value.trim().is_empty()))
                    {
                        return Err(format!("question '{id}' has a blank choice description"));
                    }
                    instructions
                },
                SystemOneQuestionConfig::Score { instructions, criteria } => {
                    if criteria.len() < 2 {
                        return Err(format!("question '{id}' must have at least two score levels"));
                    }
                    if criteria.iter().any(|level| level.trim().is_empty()) {
                        return Err(format!("question '{id}' has a blank score level"));
                    }
                    instructions
                },
            };

            if instructions.trim().is_empty() {
                return Err(format!("question '{id}' instructions must not be empty"));
            }
        }

        self.api_key_env.as_deref().map_or(Ok(()), |name| {
            let mut chars = name.chars();
            let valid_first = chars.next().is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic());
            let valid_rest = chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric());
            if valid_first && valid_rest {
                Ok(())
            } else {
                Err("'api_key_env' must be a valid environment variable name".into())
            }
        })
    }
}

/// Optional criteria for a `noul` question.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoulCriteria {
    /// Description of what counts as a positive answer.
    #[serde(rename = "true", default, skip_serializing_if = "Option::is_none")]
    pub(super) positive: Option<String>,
    /// Description of what counts as a negative answer.
    #[serde(rename = "false", default, skip_serializing_if = "Option::is_none")]
    pub(super) negative: Option<String>,
}

/// Default maximum serialized request size: 128 KiB.
fn default_request_limit_size() -> usize {
    MAX_REQUEST_BYTES // 128 KiB
}

/// Default maximum serialized response size: 64 KiB.
fn default_response_limit_size() -> usize {
    MAX_RESPONSE_BYTES // 64 KiB
}

/// Default outbound delivery timeout: 2 seconds.
fn default_timeout_ms() -> u64 {
    2_000
}
