// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Compile-time coverage for the catalog derive re-export.

extern crate praxis_proxy_config_catalog as praxis_config_catalog;

use praxis_config_catalog::{ConfigSchemaFor, SchemaId};

#[derive(praxis_config_catalog::ConfigSchemaFor)]
#[config_schema(id = "test.example")]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
struct Example {
    value_name: String,
}

#[derive(praxis_config_catalog::ConfigSchemaFor)]
#[config_schema(id = "test.mode")]
#[allow(dead_code)]
enum Mode {
    Fast,
    #[serde(rename = "slow-mode")]
    Slow,
}

#[derive(praxis_config_catalog::ConfigSchemaFor)]
#[config_schema(id = "test.tagged")]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum Tagged {
    Simple,
    Rich { value: String },
}

#[derive(praxis_config_catalog::ConfigSchemaFor)]
#[config_schema(id = "test.options")]
#[allow(dead_code)]
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
#[allow(dead_code)]
struct Extra {
    value: u32,
}

#[derive(praxis_config_catalog::ConfigSchemaFor)]
#[config_schema(id = "test.recursive")]
#[allow(dead_code)]
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
    let fields = match &schemas[&SchemaId::from("test.example")].node.kind {
        praxis_config_catalog::SchemaKind::Object { fields, .. } => fields,
        _ => panic!("expected object schema"),
    };
    assert_eq!(fields[0].serialized_name, "value-name");
}

#[test]
fn derive_preserves_field_metadata_and_cycles() {
    let mut schemas = std::collections::BTreeMap::new();
    let mut visiting = std::collections::BTreeSet::new();
    Options::register(&mut schemas, &mut visiting);
    let fields = match &schemas[&SchemaId::from("test.options")].node.kind {
        praxis_config_catalog::SchemaKind::Object { fields, .. } => fields,
        _ => panic!("expected object schema"),
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
