// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2024 Praxis Contributors

//! Deserialized YAML configuration types for the credential injection filter.

use std::fmt;

use secrecy::SecretString;
use serde::Deserialize;

// -----------------------------------------------------------------------------
// CredentialInjectionConfig
// -----------------------------------------------------------------------------

/// Deserialized YAML config for the credential injection filter.
///
/// ```yaml
/// filter: credential_injection
/// clusters:
///   - name: provider-a
///     header: Authorization
///     env_var: PROVIDER_A_API_KEY
///     header_prefix: "Bearer "
///     strip_client_credential: true
/// ```
#[derive(Debug, Deserialize, praxis_config_catalog::ConfigSchemaFor)]
#[config_schema(id = "core.filter.http.security.credential_injection")]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialInjectionConfig {
    /// Per-cluster credential injection rules.
    pub clusters: Vec<ClusterCredentialConfig>,
}

// -----------------------------------------------------------------------------
// ClusterCredentialConfig
// -----------------------------------------------------------------------------

/// Credential injection rule for a single cluster.
///
/// Exactly one of `value` or `env_var` must be set.
/// When `env_var` is used, the environment variable is
/// read once at filter construction time.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClusterCredentialConfig {
    /// Cluster name this rule applies to.
    pub name: String,

    /// Environment variable name containing the credential.
    /// Resolved at filter construction time.
    /// Mutually exclusive with `value`.
    pub env_var: Option<String>,

    /// Header name to inject (e.g. `"Authorization"`, `"x-api-key"`).
    pub header: String,

    /// Optional prefix prepended to the credential value
    /// before injection (e.g. `"Bearer "`).
    #[serde(default)]
    pub header_prefix: Option<String>,

    /// Deprecated: injection always replaces any client-provided
    /// value for the header. Retained for config compatibility.
    #[serde(default = "default_strip")]
    pub strip_client_credential: bool,

    /// Literal credential value. Mutually exclusive with `env_var`.
    /// Wrapped in [`SecretString`] to prevent accidental logging.
    pub value: Option<SecretString>,
}

impl praxis_config_catalog::ConfigSchemaFor for ClusterCredentialConfig {
    fn schema_id() -> praxis_config_catalog::SchemaId {
        praxis_config_catalog::SchemaId::from("core.credential_injection.cluster")
    }

    #[expect(clippy::too_many_lines, reason = "credential schema mirrors the wire configuration")]
    fn register(
        schemas: &mut std::collections::BTreeMap<praxis_config_catalog::SchemaId, praxis_config_catalog::ConfigSchema>,
        visiting: &mut std::collections::BTreeSet<praxis_config_catalog::SchemaId>,
    ) -> praxis_config_catalog::SchemaNode {
        use praxis_config_catalog::{ObjectField, SchemaNode};
        let mut secret = SchemaNode::simple(praxis_config_catalog::SchemaKind::String);
        secret.sensitive = true;
        SchemaNode::object(vec![
            ObjectField {
                serialized_name: "name".into(),
                aliases: vec![],
                schema: <String as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting),
                required: true,
                flattened: false,
            },
            ObjectField {
                serialized_name: "env_var".into(),
                aliases: vec![],
                schema: <Option<String> as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting),
                required: false,
                flattened: false,
            },
            ObjectField {
                serialized_name: "header".into(),
                aliases: vec![],
                schema: <String as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting),
                required: true,
                flattened: false,
            },
            ObjectField {
                serialized_name: "header_prefix".into(),
                aliases: vec![],
                schema: <Option<String> as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting),
                required: false,
                flattened: false,
            },
            ObjectField {
                serialized_name: "strip_client_credential".into(),
                aliases: vec![],
                schema: <bool as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting),
                required: false,
                flattened: false,
            },
            ObjectField {
                serialized_name: "value".into(),
                aliases: vec![],
                schema: secret,
                required: false,
                flattened: false,
            },
        ])
    }
}

impl fmt::Debug for ClusterCredentialConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.value.as_ref().map(|_| "[REDACTED]");

        f.debug_struct("ClusterCredentialConfig")
            .field("name", &self.name)
            .field("env_var", &self.env_var)
            .field("header", &self.header)
            .field("header_prefix", &self.header_prefix)
            .field("strip_client_credential", &self.strip_client_credential)
            .field("value", &value)
            .finish()
    }
}

/// Default for `strip_client_credential`.
fn default_strip() -> bool {
    true
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "tests use expect for parse setup")]
mod tests {
    use secrecy::ExposeSecret as _;

    use super::*;

    #[test]
    fn inline_credential_value_is_secret_string() {
        let cfg: CredentialInjectionConfig = serde_yaml::from_str(
            "
clusters:
  - name: provider-a
    header: Authorization
    value: super-secret-inline-value
",
        )
        .expect("credential injection config should parse");

        let value = cfg
            .clusters
            .first()
            .expect("credential config should include one cluster")
            .value
            .as_ref()
            .expect("inline credential value should be present");
        let _: &SecretString = value;
        assert_eq!(value.expose_secret(), "super-secret-inline-value");
    }

    #[test]
    fn debug_redacts_inline_credential_value() {
        let cfg: CredentialInjectionConfig = serde_yaml::from_str(
            "
clusters:
  - name: provider-a
    header: Authorization
    value: super-secret-inline-value
    header_prefix: 'Bearer '
",
        )
        .expect("credential injection config should parse");

        let debug = format!("{cfg:?}");
        assert!(
            debug.contains("REDACTED"),
            "Debug output should include redaction marker"
        );
        assert!(debug.contains("provider-a"), "Debug output should retain cluster name");
        assert!(
            !debug.contains("super-secret-inline-value"),
            "Debug output must not contain inline credential value"
        );
    }

    #[test]
    fn debug_preserves_env_var_name_without_literal_secret() {
        let cfg: CredentialInjectionConfig = serde_yaml::from_str(
            "
clusters:
  - name: provider-a
    header: Authorization
    env_var: PROVIDER_A_API_KEY
",
        )
        .expect("credential injection config should parse");

        let debug = format!("{cfg:?}");
        assert!(
            debug.contains("PROVIDER_A_API_KEY"),
            "Debug output should retain env var name"
        );
        assert!(
            !debug.contains("REDACTED"),
            "Debug output should not redact absent literal values"
        );
    }
}
