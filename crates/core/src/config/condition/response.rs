// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2024 Praxis Contributors

//! Response-phase condition predicates that gate filter execution.

use std::collections::HashMap;

use serde::Deserialize;

use super::{impl_condition_deserialize, impl_condition_serialize};

// -----------------------------------------------------------------------------
// ResponseCondition
// -----------------------------------------------------------------------------

/// Gates filter execution during the response phase.
///
/// ```
/// use praxis_core::config::ResponseCondition;
///
/// let conditions: Vec<ResponseCondition> = serde_yaml::from_str(
///     r#"
/// - when:
///     status: [200, 201]
/// - unless:
///     headers:
///       x-skip-filter: "true"
/// "#,
/// )
/// .unwrap();
/// assert_eq!(conditions.len(), 2);
/// ```
#[derive(Clone, Debug)]
pub enum ResponseCondition {
    /// Execute the filter only if the response predicate matches.
    When(ResponseConditionMatch),

    /// Skip the filter if the response predicate matches.
    Unless(ResponseConditionMatch),
}

impl_condition_deserialize!(ResponseCondition, ResponseConditionMatch, "response condition");
impl_condition_serialize!(ResponseCondition, ResponseConditionMatch);

impl praxis_config_catalog::ConfigSchemaFor for ResponseCondition {
    fn schema_id() -> praxis_config_catalog::SchemaId {
        praxis_config_catalog::SchemaId::from("core.response_condition")
    }

    #[expect(
        clippy::too_many_lines,
        reason = "condition schema mirrors the wire variants explicitly"
    )]
    fn register(
        _schemas: &mut std::collections::BTreeMap<praxis_config_catalog::SchemaId, praxis_config_catalog::ConfigSchema>,
        _visiting: &mut std::collections::BTreeSet<praxis_config_catalog::SchemaId>,
    ) -> praxis_config_catalog::SchemaNode {
        let when_fields = vec![
            praxis_config_catalog::ObjectField {
                serialized_name: "status".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::array(praxis_config_catalog::SchemaNode::simple(
                    praxis_config_catalog::SchemaKind::Integer,
                )),
                required: false,
                flattened: false,
            },
            praxis_config_catalog::ObjectField {
                serialized_name: "headers".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::map(praxis_config_catalog::SchemaNode::simple(
                    praxis_config_catalog::SchemaKind::String,
                )),
                required: false,
                flattened: false,
            },
        ];

        let when_obj = praxis_config_catalog::SchemaNode::object(when_fields);
        let when_variant = praxis_config_catalog::SchemaNode::object(vec![praxis_config_catalog::ObjectField {
            serialized_name: "when".to_owned(),
            aliases: Vec::new(),
            schema: when_obj,
            required: true,
            flattened: false,
        }]);

        let unless_fields = vec![
            praxis_config_catalog::ObjectField {
                serialized_name: "status".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::array(praxis_config_catalog::SchemaNode::simple(
                    praxis_config_catalog::SchemaKind::Integer,
                )),
                required: false,
                flattened: false,
            },
            praxis_config_catalog::ObjectField {
                serialized_name: "headers".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::map(praxis_config_catalog::SchemaNode::simple(
                    praxis_config_catalog::SchemaKind::String,
                )),
                required: false,
                flattened: false,
            },
        ];

        let unless_obj = praxis_config_catalog::SchemaNode::object(unless_fields);
        let unless_variant = praxis_config_catalog::SchemaNode::object(vec![praxis_config_catalog::ObjectField {
            serialized_name: "unless".to_owned(),
            aliases: Vec::new(),
            schema: unless_obj,
            required: true,
            flattened: false,
        }]);

        praxis_config_catalog::SchemaNode::one_of(vec![when_variant, unless_variant])
    }
}

// -----------------------------------------------------------------------------
// ResponseConditionMatch
// -----------------------------------------------------------------------------

/// Match predicate for a response condition.
///
/// ```
/// use praxis_core::config::ResponseConditionMatch;
///
/// let m: ResponseConditionMatch = serde_yaml::from_str(
///     r#"
/// status: [200, 201]
/// headers:
///   content-type: "application/json"
/// "#,
/// )
/// .unwrap();
/// assert_eq!(m.status.as_ref().unwrap(), &[200, 201]);
/// ```
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseConditionMatch {
    /// Response status code must be one of these.
    #[serde(default)]
    pub status: Option<Vec<u16>>,

    /// Response headers that must be present and match.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
#[expect(clippy::allow_attributes, reason = "blanket test suppressions")]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::needless_raw_strings,
    clippy::needless_raw_string_hashes,
    reason = "tests use unwrap/expect/indexing/raw strings for brevity"
)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_condition_when_status() {
        let yaml = r#"
- when:
    status: [200, 201]
"#;
        let conds: Vec<ResponseCondition> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(conds.len(), 1, "should parse 1 condition");
        assert!(
            matches!(
                &conds[0],
                ResponseCondition::When(m) if m.status.as_ref().unwrap() == &[200, 201]
            ),
            "should be When condition with status [200, 201]"
        );
    }

    #[test]
    fn parse_response_condition_unless_headers() {
        let yaml = r#"
- unless:
    headers:
      x-skip: "true"
"#;
        let conds: Vec<ResponseCondition> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(conds.len(), 1, "should parse 1 condition");
        assert!(
            matches!(&conds[0], ResponseCondition::Unless(m) if m.headers.is_some()),
            "should be Unless condition with headers"
        );
    }

    #[test]
    fn parse_response_condition_all_fields() {
        let yaml = r#"
status: [500, 502, 503]
headers:
  content-type: "text/html"
"#;
        let m: ResponseConditionMatch = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.status.as_ref().unwrap(), &[500, 502, 503], "status codes mismatch");
        assert_eq!(
            m.headers.as_ref().unwrap().get("content-type").unwrap(),
            "text/html",
            "content-type header mismatch"
        );
    }

    #[test]
    fn parse_empty_response_conditions() {
        let conds: Vec<ResponseCondition> = serde_yaml::from_str("[]").unwrap();
        assert!(conds.is_empty(), "empty array should parse to empty vec");
    }

    #[test]
    fn reject_response_condition_neither() {
        let yaml = "- {}";
        let err = serde_yaml::from_str::<Vec<ResponseCondition>>(yaml).unwrap_err();
        assert!(err.to_string().contains("either"));
    }

    #[test]
    fn reject_response_condition_both() {
        let yaml = r#"
- when:
    status: [200]
  unless:
    status: [500]
"#;
        let err = serde_yaml::from_str::<Vec<ResponseCondition>>(yaml).unwrap_err();
        assert!(err.to_string().contains("exactly one"));
    }
}
