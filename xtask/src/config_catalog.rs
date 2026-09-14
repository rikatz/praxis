// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Generate the browser-facing Core configuration catalog.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use clap::Parser;
use praxis_config_catalog::{
    CatalogFormatVersion, CatalogFragment, ConfigSchema, ConfigSchemaFor, ObjectField, Protocol, SchemaKind,
    SchemaNode, SecurityClass,
};

/// Arguments for catalog generation.
#[derive(Parser)]
pub(crate) struct GenerateArgs {}

/// Arguments for catalog linting.
#[derive(Parser)]
pub(crate) struct LintArgs {}

/// Generate the Core catalog release artifact.
pub(crate) fn generate(_args: GenerateArgs) {
    let root = workspace_root();
    let artifact = root.join("docs/catalog/config-catalog.json");
    let bytes = render(&root);
    if let Some(parent) = artifact.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| fail(parent, e));
    }
    fs::write(&artifact, bytes).unwrap_or_else(|e| fail(&artifact, e));
    println!("wrote {}", artifact.display());
}

/// Validate that the Core catalog can be generated.
pub(crate) fn lint(_args: LintArgs) {
    let root = workspace_root();
    let _ = render(&root);
    println!("configuration catalog generation is valid");
}

fn source_revision() -> Option<String> {
    std::env::var("PRAXIS_SOURCE_REVISION")
        .ok()
        .filter(|revision| !revision.trim().is_empty())
}

fn render(root: &Path) -> Vec<u8> {
    let shared = super::filter_docs::parse_shared_config_items(root);
    let entries = super::filter_docs::discover_all_filters(root, &shared);
    let registry = praxis_filter::FilterRegistry::with_builtins();
    let mut fragment = CatalogFragment {
        format_version: CatalogFormatVersion { major: 1, minor: 0 },
        producer: praxis_config_catalog::ProducerInfo {
            component: praxis_config_catalog::ProducerComponent::Core,
            package: "praxis-proxy-core".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            source_revision: source_revision(),
        },
        compatibility: praxis_config_catalog::CatalogCompatibility {
            requires_format_major: 1,
            requires_core: None,
        },
        feature_profile: feature_profile(root),
        schemas: Default::default(),
        roots: vec![praxis_config_catalog::RootConfigDescriptor {
            name: "Config".to_owned(),
            schema: "core.root.config".into(),
        }],
        filters: Vec::new(),
        diagnostics: Vec::new(),
    };
    <praxis_core::config::Config as ConfigSchemaFor>::register(&mut fragment.schemas, &mut BTreeSet::new());
    let root_schema = fragment
        .schemas
        .get_mut(&"core.root.config".into())
        .expect("Config derive must register core.root.config");
    root_schema.title = "Praxis configuration".to_owned();
    root_schema.description = "The top-level Praxis configuration.".to_owned();
    fragment.schemas.insert(
        "core.filter.entry.config".into(),
        ConfigSchema {
            id: "core.filter.entry.config".into(),
            title: "Filter configuration payload".to_owned(),
            description:
                "Discriminated filter payload; the selected filter supplies the referenced configuration schema."
                    .to_owned(),
            shared: false,
            node: SchemaNode {
                kind: SchemaKind::Object {
                    fields: vec![
                        ObjectField {
                            serialized_name: "filter".into(),
                            aliases: Vec::new(),
                            schema: SchemaNode::simple(SchemaKind::String),
                            required: true,
                            flattened: false,
                        },
                        ObjectField {
                            serialized_name: "config".into(),
                            aliases: Vec::new(),
                            schema: SchemaNode::map(SchemaNode::simple(SchemaKind::String)),
                            required: true,
                            flattened: false,
                        },
                    ],
                    additional_properties: false,
                },
                title: None,
                description: String::new(),
                default: None,
                examples: Vec::new(),
                rules: Vec::new(),
                sensitive: false,
            },
            producer: None,
        },
    );
    for entry in entries {
        let name = entry.filter.name.as_str();
        if !registry.available_filters().contains(&name) {
            continue;
        }
        let (schema_id, node) = registry
            .register_schema(name, &mut fragment.schemas, &mut BTreeSet::new())
            .unwrap_or_else(|| panic!("filter {name} has no typed catalog schema"));
        if !fragment.schemas.contains_key(&schema_id) {
            fragment.schemas.insert(
                schema_id.clone(),
                ConfigSchema {
                    id: schema_id.clone(),
                    title: entry.filter.name.clone(),
                    description: entry.filter.description.clone(),
                    shared: false,
                    node,
                    producer: None,
                },
            );
        }
        let mut required_features = BTreeSet::new();
        if let Some(feature) = entry.required_feature {
            required_features.insert(feature);
        }
        let filter_name = entry.filter.name;
        let protocol = if entry.protocol == "tcp" {
            Protocol::Tcp
        } else {
            Protocol::Http
        };
        let capabilities = filter_capabilities(&filter_name, &registry);
        fragment.filters.push(praxis_config_catalog::FilterDescriptor {
            name: filter_name.clone(),
            protocol,
            category: entry.category,
            description: entry.filter.description,
            config_schema: schema_id,
            required_features,
            capabilities,
            examples: entry
                .filter
                .yaml_examples
                .into_iter()
                .map(|yaml| praxis_config_catalog::ConfigExample { yaml })
                .collect(),
            source: None,
            producer: None,
        });
    }
    let descriptor_names: BTreeSet<&str> = fragment.filters.iter().map(|filter| filter.name.as_str()).collect();
    for name in registry.available_filters() {
        if !descriptor_names.contains(name) {
            panic!("registered filter is missing from the catalog: {name}");
        }
    }
    if let Some(schema) = fragment.schemas.get_mut(&"core.filter.entry.config".into()) {
        schema.node = SchemaNode::one_of(
            fragment
                .filters
                .iter()
                .map(|filter| SchemaNode::reference(filter.config_schema.0.clone()))
                .collect(),
        );
    }
    let producer = fragment.producer.clone();
    for schema in fragment.schemas.values_mut() {
        schema.producer.get_or_insert_with(|| producer.clone());
    }
    for filter in &mut fragment.filters {
        filter.producer.get_or_insert_with(|| producer.clone());
    }
    fragment.filters.sort_by(|left, right| {
        (&left.protocol, &left.category, &left.name).cmp(&(&right.protocol, &right.category, &right.name))
    });
    fragment
        .validate()
        .unwrap_or_else(|error| panic!("invalid generated catalog: {error}"));
    let mut bytes = serde_json::to_vec_pretty(&fragment).expect("catalog serialization");
    bytes.push(b'\n');
    bytes
}


fn filter_capabilities(
    name: &str,
    registry: &praxis_filter::FilterRegistry,
) -> praxis_config_catalog::FilterCapabilities {
    praxis_config_catalog::FilterCapabilities {
        security_class: if registry.is_security_filter(name) {
            SecurityClass::Security
        } else {
            SecurityClass::Standard
        },
        request_headers: false,
        request_body: false,
        response_headers: false,
        response_body: false,
        terminal: praxis_core::config::TERMINAL_FILTERS.contains(&name),
    }
}

fn feature_profile(root: &Path) -> praxis_config_catalog::FeatureProfile {
    let path = root.join("crates/filter/Cargo.toml");
    let Ok(source) = fs::read_to_string(path) else {
        return Default::default();
    };
    let mut in_features = false;
    let mut available = BTreeSet::new();
    let mut default_enabled = BTreeSet::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if !in_features || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        let name = name.trim().to_owned();
        available.insert(name.clone());
        if name == "default" {
            default_enabled.extend(
                value
                    .trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(str::trim)
                    .map(|feature| feature.trim_matches('"').to_owned())
                    .filter(|feature| !feature.is_empty()),
            );
        }
    }
    praxis_config_catalog::FeatureProfile {
        available,
        default_enabled,
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has workspace parent")
        .to_path_buf()
}
fn fail(path: &Path, error: std::io::Error) -> ! {
    panic!("{}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_catalog_is_nonempty_and_valid() {
        let bytes = render(&workspace_root());
        let fragment: CatalogFragment = serde_json::from_slice(&bytes).expect("generated JSON should parse");
        fragment.validate().expect("generated catalog should validate");
        assert_eq!(fragment.roots.len(), 1);
        assert!(!fragment.schemas.is_empty());
        assert!(!fragment.filters.is_empty());
        let root = &fragment.schemas[&fragment.roots[0].schema];
        let SchemaKind::Object { fields, .. } = &root.node.kind else {
            panic!("root schema must remain an object, not a self-reference")
        };
        assert!(fields.iter().any(|field| field.serialized_name == "listeners"));
    }

    #[test]
    fn generated_catalog_is_deterministic() {
        assert_eq!(render(&workspace_root()), render(&workspace_root()));
    }

    #[test]
    fn generated_catalog_emits_try_from_rules_and_enum_discriminators() {
        let bytes = render(&workspace_root());
        let fragment: CatalogFragment = serde_json::from_slice(&bytes).expect("generated JSON should parse");
        let retry_policy = fragment
            .schemas
            .get(&"core.retry_policy".into())
            .expect("retry policy schema");
        let retry_limit = match &retry_policy.node.kind {
            SchemaKind::Object { fields, .. } => fields
                .iter()
                .find(|field| field.serialized_name == "retry_body_limit_bytes")
                .expect("retry body limit field"),
            _ => panic!("retry policy must be an object"),
        };
        let SchemaKind::Integer = retry_limit.schema.kind else {
            panic!("retry body limit must remain an integer")
        };
        assert_eq!(retry_limit.schema.rules.len(), 1);
        assert_eq!(
            retry_limit.schema.rules[0].kind,
            praxis_config_catalog::RuleKind::NumericBounds
        );

        fn has_discriminator(node: &SchemaNode) -> bool {
            match &node.kind {
                SchemaKind::OneOf {
                    variants,
                    discriminator,
                } => discriminator.is_some() || variants.iter().any(has_discriminator),
                SchemaKind::Object { fields, .. } => fields.iter().any(|field| has_discriminator(&field.schema)),
                SchemaKind::Array { items, .. } | SchemaKind::Map { values: items } => has_discriminator(items),
                _ => false,
            }
        }
        assert!(
            fragment.schemas.values().any(|schema| has_discriminator(&schema.node)),
            "at least one tagged enum must expose its discriminator"
        );
    }

    #[test]
    fn registry_and_feature_profile_are_catalog_parity_checked() {
        let bytes = render(&workspace_root());
        let fragment: CatalogFragment = serde_json::from_slice(&bytes).expect("generated JSON should parse");
        let descriptors: BTreeSet<&str> = fragment.filters.iter().map(|filter| filter.name.as_str()).collect();
        let registry = praxis_filter::FilterRegistry::with_builtins();
        for name in registry.available_filters() {
            assert!(
                descriptors.contains(name),
                "registered filter {name} has no generated catalog descriptor"
            );
        }
        for filter in &fragment.filters {
            assert!(
                filter.required_features.is_subset(&fragment.feature_profile.available),
                "{} requires a feature outside the generated profile",
                filter.name
            );
        }
    }
}
