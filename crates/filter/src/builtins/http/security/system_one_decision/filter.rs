// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Request-side System One decision filter.

use std::collections::BTreeMap;

use async_trait::async_trait;
use bytes::Bytes;
use jev::{Answer as SystemOneAnswer, Evaluation, Usage as SystemOneTokenUsage};
use secrecy::SecretString;
use serde::Serialize;

use super::config::{MAX_BODY_BYTES, StateValueConfig, SystemOneDecisionConfig, SystemOneQuestionConfig};
use crate::{
    BodyAccess, BodyMode, FilterAction, FilterError, Rejection,
    factory::parse_filter_config,
    filter::{HttpFilter, HttpFilterContext},
};

// -----------------------------------------------------------------------------
// SystemOneDecisionFilter
// -----------------------------------------------------------------------------

/// Request-side System One filter.
///
/// Evaluates configured System One questions and keeps validated answers on the
/// request context.
///
/// # Example
///
/// The complete hosted Jev configuration is in
/// `examples/configs/security/system-one-jev.yaml`. A filter entry looks like:
///
/// ```yaml
/// - filter: system_one_decision
///   endpoint: https://api.typesafe.ai/v1/systemone
///   model: jev-1.13.0
///   api_key_env: TYPESAFE_API_KEY
///   state:
///     headers: {from: request.headers}
///     method: {from: request.method}
///     path: {from: request.path}
///   questions:
///     automated_abuse:
///       type: noul
///       instructions: "Does this request appear to be automated abuse?"
/// ```
///
/// The filter stores and debug-logs the validated answer; the answer does not
/// block or route the request.
pub struct SystemOneDecisionFilter {
    config: SystemOneDecisionConfig,
    api_key: Option<SecretString>,
}

enum SystemOneCallFailure {
    BadRequest,
    TooLarge,
    Unavailable,
    BadGateway,
    Internal,
}

impl SystemOneCallFailure {
    fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::TooLarge => 413,
            Self::Unavailable => 503,
            Self::BadGateway => 502,
            Self::Internal => 500,
        }
    }
}

impl SystemOneDecisionFilter {
    /// Parse the MVP filter configuration.
    ///
    /// # Errors
    ///
    /// Returns a config, endpoint, credential, or question validation error.
    pub fn from_config(config: &serde_yaml::Value) -> Result<Box<dyn HttpFilter>, FilterError> {
        let config: SystemOneDecisionConfig = parse_filter_config("system_one_decision", config)?;
        config
            .validate()
            .map_err(|error| -> FilterError { format!("system_one_decision: {error}").into() })?;

        let api_key = config
            .api_key_env
            .as_deref()
            .map(|name| {
                std::env::var(name)
                    .map_err(|_| -> FilterError {
                        format!("system_one_decision: api key environment variable '{name}' is unavailable").into()
                    })
                    .and_then(|value| {
                        if value.trim().is_empty() {
                            Err(format!("system_one_decision: api key environment variable '{name}' is empty").into())
                        } else {
                            Ok(SecretString::from(value))
                        }
                    })
            })
            .transpose()?;

        Ok(Box::new(Self { config, api_key }))
    }

    /// Capture request metadata, textual body, and complete header map.
    /// Header values that cannot be represented as strings must fail closed.
    fn capture_state(&self, ctx: &HttpFilterContext<'_>) -> Result<SystemOneRequestState, SystemOneCallFailure> {
        self.config
            .state
            .iter()
            .map(|(key, source)| {
                let value = match source {
                    StateValueConfig::Static(value) => serde_json::Value::String(value.clone()),
                    StateValueConfig::Source(source) => match source.from.as_str() {
                        "request.method" => serde_json::Value::String(ctx.request.method.as_str().to_owned()),
                        "request.path" => serde_json::Value::String(ctx.request.uri.path().to_owned()),
                        "request.query" => {
                            serde_json::Value::String(ctx.request.uri.query().unwrap_or_default().to_owned())
                        },
                        "request.headers" => header_state(&ctx.request.headers)?,
                        "request.body" => {
                            let body = ctx
                                .buffered_request_body
                                .as_deref()
                                .ok_or(SystemOneCallFailure::Internal)?;
                            let body = std::str::from_utf8(body).map_err(|_| SystemOneCallFailure::BadRequest)?;
                            serde_json::Value::String(body.to_owned())
                        },
                        "client.ip" => serde_json::Value::String(
                            ctx.client_addr.ok_or(SystemOneCallFailure::Internal)?.to_string(),
                        ),
                        _ => {
                            return Err(SystemOneCallFailure::Internal);
                        },
                    },
                };
                Ok((key.clone(), value))
            })
            .collect()
    }

    /// Assemble the local `{model, state, questions}` wire envelope.
    fn build_wire_request(&self, state: SystemOneRequestState) -> Result<SystemOneWireRequest, FilterError> {
        Ok(SystemOneWireRequest {
            model: self.config.model.clone(),
            state,
            questions: self.config.questions.clone(),
        })
    }

    /// Send one bounded request through Praxis subrequest transport.
    async fn execute_request(
        &self,
        ctx: &HttpFilterContext<'_>,
        request: &SystemOneWireRequest,
    ) -> Result<serde_json::Value, SystemOneCallFailure> {
        use std::time::{Duration, Instant};

        use praxis_core::{
            connectivity::prepare_url_target,
            subrequest::{FrameworkHeaders, SubRequest, SubRequestError},
        };
        use secrecy::ExposeSecret;

        let body = serde_json::to_vec(request).map_err(|_| SystemOneCallFailure::BadGateway)?;
        if body.len() > self.config.request_size_limit_bytes {
            return Err(SystemOneCallFailure::TooLarge);
        }

        let deadline = Instant::now()
            .checked_add(Duration::from_millis(self.config.request_timeout_ms))
            .ok_or(SystemOneCallFailure::Unavailable)?;

        // The endpoint is fixed in operator configuration. This pins the resolved
        // address for the request; use an address validator if that config isn't trusted.
        let target = prepare_url_target(&self.config.endpoint, deadline, |_| Ok(()))
            .await
            .map_err(|_| SystemOneCallFailure::Unavailable)?;

        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        headers.insert(http::header::ACCEPT, http::HeaderValue::from_static("application/json"));

        if let Some(api_key) = &self.api_key {
            let value = format!("Bearer {}", api_key.expose_secret());
            let value = http::HeaderValue::from_str(&value).map_err(|_| SystemOneCallFailure::BadGateway)?;
            headers.insert(http::header::AUTHORIZATION, value);
        }

        let request = target.bind(SubRequest {
            method: http::Method::POST,
            uri: http::Uri::from_static("/"),
            headers,
            body: Bytes::from(body),
        });

        let peer = request.peers().next().ok_or(SystemOneCallFailure::Unavailable)?;
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err(SystemOneCallFailure::Unavailable);
        }

        let response = ctx
            .execute_subrequest(
                &peer,
                request.request(),
                self.config.response_size_limit_bytes,
                timeout,
                FrameworkHeaders::new(),
            )
            .await
            .map_err(|error| match error {
                SubRequestError::ResponseTooLarge { .. } => SystemOneCallFailure::BadGateway,
                SubRequestError::InvalidRequest(_)
                | SubRequestError::AdmissionTimeout { .. }
                | SubRequestError::Connect(_)
                | SubRequestError::Io(_)
                | SubRequestError::DeadlineExceeded
                | SubRequestError::StreamIdleTimeout { .. }
                | SubRequestError::CircuitOpen { .. } => SystemOneCallFailure::Unavailable,
                _ => SystemOneCallFailure::Unavailable,
            })?;

        match response.status {
            200..=299 => serde_json::from_slice(&response.body).map_err(|_| SystemOneCallFailure::BadGateway),
            429 | 503 | 529 => Err(SystemOneCallFailure::Unavailable),
            _ => Err(SystemOneCallFailure::BadGateway),
        }
    }

    /// Decode and validate one evaluation against the configured questions.
    fn decode_and_validate(&self, response: serde_json::Value) -> Result<SystemOneDecision, SystemOneCallFailure> {
        let object = response.as_object().ok_or(SystemOneCallFailure::BadGateway)?;
        let answers = object
            .get("answers")
            .and_then(serde_json::Value::as_object)
            .ok_or(SystemOneCallFailure::BadGateway)?;

        if answers.len() != self.config.questions.len()
            || self.config.questions.keys().any(|id| !answers.contains_key(id))
        {
            return Err(SystemOneCallFailure::BadGateway);
        }

        let reported_model = match object.get("model") {
            Some(serde_json::Value::String(model)) if !model.trim().is_empty() => Some(model.clone()),
            Some(_) => return Err(SystemOneCallFailure::BadGateway),
            None => None,
        };
        let usage_present = match object.get("usage") {
            Some(serde_json::Value::Object(usage)) => {
                if !usage.get("input_tokens").and_then(serde_json::Value::as_u64).is_some()
                    || !usage.get("output_tokens").and_then(serde_json::Value::as_u64).is_some()
                {
                    return Err(SystemOneCallFailure::BadGateway);
                }
                true
            },
            Some(_) => return Err(SystemOneCallFailure::BadGateway),
            None => false,
        };

        for (id, question) in &self.config.questions {
            validate_raw_answer(question, answers.get(id).ok_or(SystemOneCallFailure::BadGateway)?)?;
        }

        let evaluation: Evaluation = serde_json::from_value(response).map_err(|_| SystemOneCallFailure::BadGateway)?;
        for (id, question) in &self.config.questions {
            let answer = evaluation.answers.get(id).ok_or(SystemOneCallFailure::BadGateway)?;
            let matches = matches!(
                (question, answer),
                (SystemOneQuestionConfig::Noul { .. }, SystemOneAnswer::Noul { .. })
                    | (SystemOneQuestionConfig::Choice { .. }, SystemOneAnswer::Choice { .. })
                    | (SystemOneQuestionConfig::Score { .. }, SystemOneAnswer::Score { .. })
            );
            if !matches {
                return Err(SystemOneCallFailure::BadGateway);
            }
        }

        let Evaluation { answers, usage, .. } = evaluation;
        Ok(SystemOneDecision {
            requested_model: self.config.model.clone(),
            reported_model,
            answers,
            usage: usage_present.then_some(usage),
        })
    }
}

#[async_trait]
impl HttpFilter for SystemOneDecisionFilter {
    fn name(&self) -> &'static str {
        "system_one_decision"
    }

    fn request_body_access(&self) -> BodyAccess {
        if self.needs_request_body() {
            BodyAccess::ReadOnly
        } else {
            BodyAccess::None
        }
    }

    fn request_body_mode(&self) -> BodyMode {
        if self.needs_request_body() {
            BodyMode::StreamBuffer {
                max_bytes: Some(MAX_BODY_BYTES),
            }
        } else {
            BodyMode::Stream
        }
    }

    async fn on_request_body(
        &self,
        ctx: &mut HttpFilterContext<'_>,
        body: &mut Option<Bytes>,
        _end_of_stream: bool,
    ) -> Result<FilterAction, FilterError> {
        let limit = u64::try_from(MAX_BODY_BYTES).unwrap_or(u64::MAX);
        let current_chunk = body.as_ref().map_or(0, Bytes::len);
        let current_chunk = u64::try_from(current_chunk).unwrap_or(u64::MAX);
        if self.needs_request_body() && ctx.request_body_bytes.saturating_add(current_chunk) > limit {
            return Ok(FilterAction::Reject(Rejection::status(413)));
        }
        Ok(FilterAction::Continue)
    }

    async fn on_request(&self, ctx: &mut HttpFilterContext<'_>) -> Result<FilterAction, FilterError> {
        if self.needs_request_body()
            && ctx
                .buffered_request_body
                .as_ref()
                .is_some_and(|body| body.len() > MAX_BODY_BYTES)
        {
            return Ok(FilterAction::Reject(Rejection::status(413)));
        }

        let state = match self.capture_state(ctx) {
            Ok(state) => state,
            Err(failure) => return Ok(FilterAction::Reject(Rejection::status(failure.status()))),
        };
        let request = self.build_wire_request(state)?;
        let response = match self.execute_request(ctx, &request).await {
            Ok(response) => response,
            Err(failure) => return Ok(FilterAction::Reject(Rejection::status(failure.status()))),
        };
        let decision = match self.decode_and_validate(response) {
            Ok(decision) => decision,
            Err(failure) => return Ok(FilterAction::Reject(Rejection::status(failure.status()))),
        };
        tracing::debug!(
            target: "system_one_decision.filter",
            model = %decision.requested_model,
            reported_model = ?decision.reported_model,
            answers = ?decision.answers,
            usage = ?decision.usage,
            "System One decision received",
        );
        ctx.extensions.insert(decision);
        Ok(FilterAction::Continue)
    }
}

impl SystemOneDecisionFilter {
    fn needs_request_body(&self) -> bool {
        self.config
            .state
            .values()
            .any(|value| matches!(value, StateValueConfig::Source(source) if source.from == "request.body"))
    }
}

fn header_state(headers: &http::HeaderMap) -> Result<serde_json::Value, SystemOneCallFailure> {
    let mut values = BTreeMap::<String, Vec<String>>::new();
    for (name, value) in headers {
        let value = value.to_str().map_err(|_| SystemOneCallFailure::BadRequest)?;
        values
            .entry(name.as_str().to_owned())
            .or_default()
            .push(value.to_owned());
    }

    let mut object = serde_json::Map::new();
    for (name, values) in values {
        let values = values.into_iter().map(serde_json::Value::String).collect();
        object.insert(name, serde_json::Value::Array(values));
    }
    Ok(serde_json::Value::Object(object))
}

fn validate_raw_answer(
    question: &SystemOneQuestionConfig,
    raw: &serde_json::Value,
) -> Result<(), SystemOneCallFailure> {
    let answer = raw.as_object().ok_or(SystemOneCallFailure::BadGateway)?;
    let expected_type = match question {
        SystemOneQuestionConfig::Noul { .. } => "noul",
        SystemOneQuestionConfig::Choice { .. } => "choice",
        SystemOneQuestionConfig::Score { .. } => "score",
    };
    if answer
        .get("type")
        .is_some_and(|value| value.as_str() != Some(expected_type))
    {
        return Err(SystemOneCallFailure::BadGateway);
    }

    let valid_probability = |value: &serde_json::Value| {
        value
            .as_f64()
            .is_some_and(|number| number.is_finite() && (0.0..=1.0).contains(&number))
    };
    let valid_distribution = |distribution: &serde_json::Map<String, serde_json::Value>, keys: &[String]| {
        distribution.len() == keys.len()
            && keys
                .iter()
                .all(|key| distribution.get(key).is_some_and(valid_probability))
            && (distribution.values().filter_map(serde_json::Value::as_f64).sum::<f64>() - 1.0).abs() <= 0.02
    };

    match question {
        SystemOneQuestionConfig::Noul { .. } => {
            if !answer.get("noul").is_some_and(valid_probability) {
                return Err(SystemOneCallFailure::BadGateway);
            }
        },
        SystemOneQuestionConfig::Choice { criteria, .. } => {
            let choice = answer
                .get("choice")
                .and_then(serde_json::Value::as_str)
                .ok_or(SystemOneCallFailure::BadGateway)?;
            let probabilities = answer
                .get("probabilities")
                .and_then(serde_json::Value::as_object)
                .ok_or(SystemOneCallFailure::BadGateway)?;
            let confidence = answer.get("confidence").ok_or(SystemOneCallFailure::BadGateway)?;
            let keys: Vec<_> = criteria.keys().cloned().collect();
            let chosen_probability = probabilities.get(choice).and_then(serde_json::Value::as_f64);
            let maximum = probabilities
                .values()
                .filter_map(serde_json::Value::as_f64)
                .reduce(f64::max);
            if !criteria.contains_key(choice)
                || !valid_probability(confidence)
                || !valid_distribution(probabilities, &keys)
                || chosen_probability.zip(maximum).is_none_or(|(chosen, max)| chosen < max)
            {
                return Err(SystemOneCallFailure::BadGateway);
            }
        },
        SystemOneQuestionConfig::Score { criteria, .. } => {
            let expected_legend: BTreeMap<_, _> = criteria
                .iter()
                .enumerate()
                .map(|(index, criterion)| (index.to_string(), criterion.clone()))
                .collect();
            let legend: BTreeMap<_, _> = answer
                .get("legend")
                .and_then(serde_json::Value::as_object)
                .ok_or(SystemOneCallFailure::BadGateway)?
                .iter()
                .map(|(key, value)| {
                    value
                        .as_str()
                        .map(|value| (key.clone(), value.to_owned()))
                        .ok_or(SystemOneCallFailure::BadGateway)
                })
                .collect::<Result<_, _>>()?;
            let probabilities = answer
                .get("probabilities")
                .and_then(serde_json::Value::as_object)
                .ok_or(SystemOneCallFailure::BadGateway)?;
            let confidence = answer.get("confidence").ok_or(SystemOneCallFailure::BadGateway)?;
            let keys: Vec<_> = expected_legend.keys().cloned().collect();
            let score = answer
                .get("score")
                .and_then(serde_json::Value::as_f64)
                .filter(|score| score.is_finite());
            let expected_score = probabilities
                .iter()
                .map(|(key, value)| {
                    key.parse::<usize>()
                        .ok()
                        .zip(value.as_f64())
                        .map(|(index, probability)| index as f64 * probability)
                })
                .collect::<Option<Vec<_>>>()
                .map(|values| values.into_iter().sum::<f64>());
            if legend != expected_legend
                || !valid_probability(confidence)
                || !valid_distribution(probabilities, &keys)
                || score.is_none_or(|score| score < 0.0 || score > (criteria.len() - 1) as f64)
                || score
                    .zip(expected_score)
                    .is_none_or(|(score, expected)| (score - expected).abs() > 0.02)
            {
                return Err(SystemOneCallFailure::BadGateway);
            }
        },
    }
    Ok(())
}

/// Dynamically configured state fields sent as one JSON object.
pub(super) type SystemOneRequestState = BTreeMap<String, serde_json::Value>;

/// Local JSON request envelope; `jev::State` cannot represent arbitrary nested
/// JSON state through its public text/facts API.
#[derive(Clone, Debug, Serialize)]
pub(super) struct SystemOneWireRequest {
    /// Requested System One model name.
    pub(super) model: String,
    /// Per-request state.
    pub(super) state: SystemOneRequestState,
    /// Static configured questions.
    pub(super) questions: BTreeMap<String, SystemOneQuestionConfig>,
}

/// Validated System One outputs stored in [`RequestExtensions`](crate::RequestExtensions).
///
/// A later filter can retrieve this value with
/// `ctx.extensions.get::<SystemOneDecision>()`. The built-in request `conditions`
/// predicates do not inspect extensions; a filter that consumes these answers
/// must read this typed value directly.
#[derive(Clone, Debug)]
pub(crate) struct SystemOneDecision {
    /// Model requested by Praxis.
    pub(crate) requested_model: String,
    /// Optional model label reported by a compatible backend.
    pub(crate) reported_model: Option<String>,
    /// Validated answers keyed by the configured question IDs.
    pub(crate) answers: BTreeMap<String, SystemOneAnswer>,
    /// Token counts when the backend included usage in its response.
    pub(crate) usage: Option<SystemOneTokenUsage>,
}
