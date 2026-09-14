// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2024 Praxis Contributors

//! Request-phase condition predicates that gate filter execution.

use std::collections::HashMap;

use serde::Deserialize;

use super::{impl_condition_deserialize, impl_condition_serialize};

// -----------------------------------------------------------------------------
// Condition
// -----------------------------------------------------------------------------

/// Gates filter execution: `When` requires a match, `Unless` skips on match.
///
/// ```
/// use praxis_core::config::Condition;
///
/// let conditions: Vec<Condition> = serde_yaml::from_str(
///     r#"
/// - when:
///     path_prefix: "/api"
/// - unless:
///     methods: ["OPTIONS"]
/// "#,
/// )
/// .unwrap();
/// assert_eq!(conditions.len(), 2);
/// ```
#[derive(Clone, Debug)]
pub enum Condition {
    /// Execute the filter only if the predicate matches.
    When(ConditionMatch),

    /// Skip the filter if the predicate matches.
    Unless(ConditionMatch),
}

impl_condition_deserialize!(Condition, ConditionMatch, "condition");
impl_condition_serialize!(Condition, ConditionMatch);

impl praxis_config_catalog::ConfigSchemaFor for Condition {
    fn schema_id() -> praxis_config_catalog::SchemaId {
        praxis_config_catalog::SchemaId::from("core.condition")
    }

    fn register(
        _schemas: &mut std::collections::BTreeMap<praxis_config_catalog::SchemaId, praxis_config_catalog::ConfigSchema>,
        _visiting: &mut std::collections::BTreeSet<praxis_config_catalog::SchemaId>,
    ) -> praxis_config_catalog::SchemaNode {
        let when_fields = vec![
            praxis_config_catalog::ObjectField {
                serialized_name: "path".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::simple(praxis_config_catalog::SchemaKind::String),
                required: false,
                flattened: false,
            },
            praxis_config_catalog::ObjectField {
                serialized_name: "path_prefix".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::simple(praxis_config_catalog::SchemaKind::String),
                required: false,
                flattened: false,
            },
            praxis_config_catalog::ObjectField {
                serialized_name: "methods".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::array(praxis_config_catalog::SchemaNode::simple(
                    praxis_config_catalog::SchemaKind::String,
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
                serialized_name: "path".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::simple(praxis_config_catalog::SchemaKind::String),
                required: false,
                flattened: false,
            },
            praxis_config_catalog::ObjectField {
                serialized_name: "path_prefix".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::simple(praxis_config_catalog::SchemaKind::String),
                required: false,
                flattened: false,
            },
            praxis_config_catalog::ObjectField {
                serialized_name: "methods".to_owned(),
                aliases: Vec::new(),
                schema: praxis_config_catalog::SchemaNode::array(praxis_config_catalog::SchemaNode::simple(
                    praxis_config_catalog::SchemaKind::String,
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
// ConditionMatch
// -----------------------------------------------------------------------------

/// Match predicate for a condition (AND semantics).
///
/// ```
/// use praxis_core::config::ConditionMatch;
///
/// let m: ConditionMatch = serde_yaml::from_str(
///     r#"
/// path_prefix: "/api"
/// methods: ["GET", "POST"]
/// "#,
/// )
/// .unwrap();
/// assert_eq!(m.path_prefix.as_deref(), Some("/api"));
/// assert_eq!(m.methods.as_ref().unwrap().len(), 2);
/// ```
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionMatch {
    /// Request URI must match this exact path.
    #[serde(default)]
    pub path: Option<String>,

    /// Request URI must match this prefix at a segment boundary.
    /// `/api` matches `/api`, `/api/`, `/api/v1` but NOT `/apikeys`.
    #[serde(default)]
    pub path_prefix: Option<String>,

    /// Request method must be one of these (case-insensitive).
    #[serde(default)]
    pub methods: Option<Vec<String>>,

    /// Headers that must be present and match.
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
    fn parse_condition_match_all_fields() {
        let yaml = r#"
path_prefix: "/api"
methods: ["GET", "POST"]
headers:
  x-tenant: "acme"
  x-debug: "true"
"#;
        let m: ConditionMatch = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.path_prefix.as_deref(), Some("/api"), "path_prefix mismatch");
        let methods = m.methods.unwrap();
        assert_eq!(methods, vec!["GET", "POST"], "methods mismatch");
        let headers = m.headers.unwrap();
        assert_eq!(headers.get("x-tenant").unwrap(), "acme", "x-tenant header mismatch");
        assert_eq!(headers.get("x-debug").unwrap(), "true", "x-debug header mismatch");
    }

    #[test]
    fn parse_condition_match_partial() {
        let yaml = r#"
path_prefix: "/health"
"#;
        let m: ConditionMatch = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.path_prefix.as_deref(), Some("/health"), "path_prefix mismatch");
        assert!(m.methods.is_none(), "methods should be None when omitted");
        assert!(m.headers.is_none(), "headers should be None when omitted");
    }

    #[test]
    fn parse_when_condition() {
        let yaml = r#"
- when:
    path_prefix: "/api"
"#;
        let conditions: Vec<Condition> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(conditions.len(), 1, "should parse 1 condition");
        assert!(
            matches!(&conditions[0], Condition::When(m) if m.path_prefix.as_deref() == Some("/api")),
            "should be When with /api prefix"
        );
    }

    #[test]
    fn parse_unless_condition() {
        let yaml = r#"
- unless:
    methods: ["OPTIONS"]
"#;
        let conditions: Vec<Condition> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(conditions.len(), 1, "should parse 1 condition");
        assert!(
            matches!(&conditions[0], Condition::Unless(m) if m.methods.as_ref().unwrap() == &["OPTIONS"]),
            "should be Unless with OPTIONS method"
        );
    }

    #[test]
    fn parse_mixed_conditions() {
        let yaml = r#"
- when:
    path_prefix: "/api"
- unless:
    headers:
      x-internal: "true"
- when:
    methods: ["POST", "PUT", "DELETE"]
"#;
        let conditions: Vec<Condition> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(conditions.len(), 3, "should parse 3 conditions");
        assert!(matches!(&conditions[0], Condition::When(_)), "first should be When");
        assert!(
            matches!(&conditions[1], Condition::Unless(_)),
            "second should be Unless"
        );
        assert!(matches!(&conditions[2], Condition::When(_)), "third should be When");
    }

    #[test]
    fn parse_empty_conditions() {
        let conditions: Vec<Condition> = serde_yaml::from_str("[]").unwrap();
        assert!(conditions.is_empty(), "empty array should parse to empty vec");
    }

    #[test]
    fn reject_both_when_and_unless() {
        let yaml = r#"
- when:
    path_prefix: "/api"
  unless:
    methods: ["GET"]
"#;
        let err = serde_yaml::from_str::<Vec<Condition>>(yaml).unwrap_err();
        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn reject_neither_when_nor_unless() {
        let yaml = "- {}";
        let err = serde_yaml::from_str::<Vec<Condition>>(yaml).unwrap_err();
        assert!(err.to_string().contains("either"));
    }

    #[test]
    fn parse_exact_path_condition() {
        let m: ConditionMatch = serde_yaml::from_str(
            r#"
path: "/"
"#,
        )
        .unwrap();
        assert_eq!(m.path.as_deref(), Some("/"), "exact path should be /");
        assert!(
            m.path_prefix.is_none(),
            "path_prefix should be None for exact path match"
        );
    }
}
