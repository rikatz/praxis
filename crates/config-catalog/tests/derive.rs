// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Compile-time coverage for the catalog derive re-export.

#![expect(
    clippy::indexing_slicing,
    clippy::panic,
    reason = "schema assertions intentionally use direct test diagnostics"
)]

extern crate praxis_proxy_config_catalog as praxis_config_catalog;

#[cfg(test)]
mod tests {
    use praxis_config_catalog::{ConfigSchemaFor as _, SchemaId};

    #[derive(praxis_config_catalog::ConfigSchemaFor)]
    #[config_schema(id = "test.example")]
    #[serde(rename_all = "kebab-case")]
    #[expect(dead_code, reason = "derive fixture is consumed through schema registration")]
    struct Example {
        value_name: String,
    }

    #[derive(praxis_config_catalog::ConfigSchemaFor)]
    #[config_schema(id = "test.mode")]
    #[expect(dead_code, reason = "derive fixture is consumed through schema registration")]
    enum Mode {
        Fast,
        #[serde(rename = "slow-mode")]
        Slow,
    }

    #[derive(praxis_config_catalog::ConfigSchemaFor)]
    #[config_schema(id = "test.tagged")]
    #[serde(tag = "type", rename_all = "snake_case")]
    #[expect(dead_code, reason = "derive fixture is consumed through schema registration")]
    enum Tagged {
        Simple,
        Rich { value: String },
    }

    #[derive(praxis_config_catalog::ConfigSchemaFor)]
    #[config_schema(id = "test.options")]
    #[expect(dead_code, reason = "derive fixture is consumed through schema registration")]
    struct Options {
        #[serde(default, alias = "old_name")]
        name: String,
        #[serde(skip)]
        internal: String,
        #[serde(flatten)]
        extra: Extra,
    }

    #[derive(praxis_config_catalog::ConfigSchemaFor)]
    #[config_schema(id = "test.extra")]
    #[expect(dead_code, reason = "derive fixture is consumed through schema registration")]
    struct Extra {
        value: u32,
    }

    #[derive(praxis_config_catalog::ConfigSchemaFor)]
    #[config_schema(id = "test.recursive")]
    #[expect(dead_code, reason = "derive fixture is consumed through schema registration")]
    struct Recursive {
        next: Option<Box<Recursive>>,
    }

    #[test]
    fn derive_registers_a_stable_id() {
        assert_eq!(Example::schema_id(), SchemaId::from("test.example"));

        let mut schemas = std::collections::BTreeMap::new();
        let mut visiting = std::collections::BTreeSet::new();
        let node = Mode::register(&mut schemas, &mut visiting);
        assert!(matches!(node.kind, praxis_config_catalog::SchemaKind::Reference { .. }));
        assert!(schemas.contains_key(&SchemaId::from("test.mode")));

        Example::register(&mut schemas, &mut visiting);
        let praxis_config_catalog::SchemaKind::Object { fields, .. } =
            &schemas[&SchemaId::from("test.example")].node.kind
        else {
            panic!("expected object schema")
        };
        assert_eq!(fields[0].serialized_name, "value-name");
    }

    #[test]
    fn derive_preserves_field_metadata_and_cycles() {
        let mut schemas = std::collections::BTreeMap::new();
        let mut visiting = std::collections::BTreeSet::new();
        Options::register(&mut schemas, &mut visiting);
        let praxis_config_catalog::SchemaKind::Object { fields, .. } =
            &schemas[&SchemaId::from("test.options")].node.kind
        else {
            panic!("expected object schema")
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].aliases, vec!["old_name"]);
        assert!(!fields[0].required);
        assert!(fields[1].flattened);

        Recursive::register(&mut schemas, &mut visiting);
        assert!(schemas.contains_key(&SchemaId::from("test.recursive")));
    }

    #[test]
    fn derive_emits_tagged_union_discriminator() {
        let mut schemas = std::collections::BTreeMap::new();
        let mut visiting = std::collections::BTreeSet::new();
        Tagged::register(&mut schemas, &mut visiting);
        let node = &schemas[&SchemaId::from("test.tagged")].node;
        let praxis_config_catalog::SchemaKind::OneOf {
            discriminator,
            variants,
        } = &node.kind
        else {
            panic!("expected tagged union")
        };
        assert_eq!(discriminator.as_deref(), Some("type"));
        assert_eq!(variants.len(), 2);
    }
}
