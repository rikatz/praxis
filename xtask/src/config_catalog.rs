//! Generate the browser-facing Core configuration catalog.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use clap::Parser;
use praxis_config_catalog::{
    CatalogFormatVersion, CatalogFragment, ConfigSchema, ObjectField, Protocol, SchemaKind, SchemaNode, SecurityClass,
};
use praxis_config_catalog_generator::{SchemaPolicy, build_fields};

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
    let (root_items, root_fields) = super::filter_docs::parse_root_config_source(root);
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
    let mut matched_overrides = BTreeSet::new();
    let root_policy = CoreSchemaPolicy::new(root, "core.root.config");
    let root_schema_fields = build_fields(
        &root_fields
            .iter()
            .filter(|field| !field.skip)
            .cloned()
            .collect::<Vec<_>>()
            .iter()
            .map(|field| root_items.source_model_field(field))
            .collect::<Vec<_>>(),
        &root_items.source_model(),
        &mut fragment.schemas,
        &mut fragment.diagnostics,
        "core.root.config",
        &root_policy,
    )
    .unwrap_or_else(|e| panic!("cannot represent root Config: {e}"));
    matched_overrides.extend(root_policy.matched.borrow().iter().copied());
    fragment.schemas.insert(
        "core.root.config".into(),
        ConfigSchema {
            id: "core.root.config".into(),
            title: "Praxis configuration".to_owned(),
            description: "The top-level Praxis configuration.".to_owned(),
            shared: false,
            node: SchemaNode {
                kind: SchemaKind::Object {
                    fields: root_schema_fields,
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
        let registry_names = registry.available_filters();
        let active = entry
            .required_feature
            .as_deref()
            .is_none_or(|feature| enabled_features(root).contains(feature));
        if active && !registry_names.contains(&entry.filter.name.as_str()) {
            panic!(
                "catalog filter is not present in the built-in registry: {}",
                entry.filter.name
            );
        }
        let schema_id = praxis_config_catalog::SchemaId(format!(
            "core.filter.{}.{}.{}",
            entry.protocol, entry.category, entry.filter.name
        ));
        let source = entry.filter.source_items.source_model();
        let fields = entry.filter.source_items.source_model_fields(&entry.filter.raw_fields);
        let policy = CoreSchemaPolicy::new(root, &schema_id.0);
        let fields = build_fields(
            &fields,
            &source,
            &mut fragment.schemas,
            &mut fragment.diagnostics,
            &schema_id.0,
            &policy,
        )
        .unwrap_or_else(|e| panic!("cannot represent filter {}: {e}", entry.filter.name));
        matched_overrides.extend(policy.matched.borrow().iter().copied());
        fragment.schemas.insert(
            schema_id.clone(),
            ConfigSchema {
                id: schema_id.clone(),
                title: entry.filter.name.clone(),
                description: entry.filter.description.clone(),
                shared: false,
                node: SchemaNode {
                    kind: SchemaKind::Object {
                        fields,
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
        let mut required_features = BTreeSet::new();
        if let Some(feature) = entry.required_feature.as_ref() {
            required_features.insert(feature.clone());
        }
        let filter_name = entry.filter.name.clone();
        let source = entry.filter.source_path.as_deref().map(|path| {
            let path = Path::new(path)
                .strip_prefix(root)
                .unwrap_or_else(|_| Path::new(path))
                .to_string_lossy()
                .replace('\\', "/");
            praxis_config_catalog::SourceLocation {
                line: source_line(entry.filter.source_path.as_deref()),
                path,
            }
        });
        let capabilities = filter_capabilities(&entry, &registry);
        fragment.filters.push(praxis_config_catalog::FilterDescriptor {
            name: filter_name.clone(),
            protocol: if entry.protocol == "http" {
                Protocol::Http
            } else {
                Protocol::Tcp
            },
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
            source,
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
    validate_examples(&fragment).unwrap_or_else(|error| panic!("invalid generated example: {error}"));
    validate_overrides(SEMANTIC_OVERRIDES, &matched_overrides).unwrap_or_else(|error| panic!("{error}"));
    praxis_config_catalog::generator::render(fragment)
        .unwrap_or_else(|error| panic!("invalid generated catalog: {error}"))
}

fn validate_examples(fragment: &CatalogFragment) -> Result<(), String> {
    for filter in &fragment.filters {
        let schema = fragment
            .schemas
            .get(&filter.config_schema)
            .ok_or_else(|| format!("example references missing schema {}", filter.config_schema))?;
        for example in &filter.examples {
            let mut value: serde_yaml::Value = serde_yaml::from_str(&example.yaml)
                .map_err(|error| format!("invalid YAML example for {}: {error}", filter.name))?;
            let serde_yaml::Value::Mapping(mapping) = &mut value else {
                return Err(format!("example for {} must be a mapping", filter.name));
            };
            let key = serde_yaml::Value::String("filter".to_owned());
            let declared = mapping.remove(&key).and_then(|value| value.as_str().map(str::to_owned));
            if declared.as_deref() != Some(filter.name.as_str()) {
                return Err(format!("example filter discriminator does not match {}", filter.name));
            }
            validate_yaml_node(&value, &schema.node, fragment, &filter.name)?;
        }
    }
    Ok(())
}

fn validate_yaml_node(
    value: &serde_yaml::Value,
    node: &SchemaNode,
    fragment: &CatalogFragment,
    target: &str,
) -> Result<(), String> {
    match &node.kind {
        SchemaKind::Reference { schema_id } => {
            let schema = fragment
                .schemas
                .get(schema_id)
                .ok_or_else(|| format!("{target} references missing schema {schema_id}"))?;
            return validate_yaml_node(value, &schema.node, fragment, target);
        },
        SchemaKind::Object {
            fields,
            additional_properties,
        } => {
            let serde_yaml::Value::Mapping(mapping) = value else {
                return Err(format!("example for {target} must be an object"));
            };
            for field in fields {
                let names = std::iter::once(&field.serialized_name).chain(field.aliases.iter());
                let matched = names
                    .filter_map(|name| mapping.get(serde_yaml::Value::String(name.clone())))
                    .next();
                if field.required && matched.is_none() {
                    return Err(format!(
                        "example for {target} is missing required field {}",
                        field.serialized_name
                    ));
                }
                if let Some(value) = matched {
                    validate_yaml_node(value, &field.schema, fragment, target)?;
                }
            }
            let flattened: Vec<&ObjectField> = fields.iter().filter(|field| field.flattened).collect();
            if !flattened.is_empty() {
                let known: BTreeSet<&str> = fields
                    .iter()
                    .flat_map(|field| {
                        std::iter::once(field.serialized_name.as_str()).chain(field.aliases.iter().map(String::as_str))
                    })
                    .collect();
                let remainder = mapping
                    .iter()
                    .filter(|(key, _)| key.as_str().is_some_and(|name| !known.contains(name)))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                let remainder = serde_yaml::Value::Mapping(remainder);
                if !matches!(&remainder, serde_yaml::Value::Mapping(values) if values.is_empty())
                    && !flattened
                        .iter()
                        .any(|field| validate_yaml_node(&remainder, &field.schema, fragment, target).is_ok())
                {
                    return Err(format!("example for {target} does not match flattened fields"));
                }
            }
            if !additional_properties {
                for key in mapping.keys() {
                    let Some(name) = key.as_str() else {
                        return Err(format!("example for {target} has a non-string field name"));
                    };
                    if flattened.is_empty()
                        && !fields.iter().any(|field| {
                            field.serialized_name == name || field.aliases.iter().any(|alias| alias == name)
                        })
                    {
                        return Err(format!("example for {target} has unknown field {name}"));
                    }
                }
            }
        },
        SchemaKind::Array {
            items,
            min_items,
            max_items,
        } => {
            let serde_yaml::Value::Sequence(values) = value else {
                return Err(format!("example for {target} must be an array"));
            };
            if min_items.is_some_and(|min| values.len() < min) || max_items.is_some_and(|max| values.len() > max) {
                return Err(format!("example for {target} has an invalid array length"));
            }
            for value in values {
                validate_yaml_node(value, items, fragment, target)?;
            }
        },
        SchemaKind::Map { values } => {
            let serde_yaml::Value::Mapping(mapping) = value else {
                return Err(format!("example for {target} must be a map"));
            };
            for (key, value) in mapping {
                if key.as_str().is_none() {
                    return Err(format!("example for {target} has a non-string map key"));
                }
                validate_yaml_node(value, values, fragment, target)?;
            }
        },
        SchemaKind::OneOf { variants, .. } => {
            if !variants
                .iter()
                .any(|variant| validate_yaml_node(value, variant, fragment, target).is_ok())
            {
                return Err(format!("example for {target} matches no schema variant"));
            }
        },
        SchemaKind::Any => return Err(format!("example for {target} uses unresolved Any")),
        SchemaKind::Boolean => {
            if !value.is_bool() {
                return Err(format!("example for {target} must be a boolean"));
            }
        },
        SchemaKind::Integer => {
            if value.as_i64().is_none() && value.as_u64().is_none() {
                return Err(format!("example for {target} must be an integer"));
            }
        },
        SchemaKind::Number => {
            if value.as_f64().is_none() {
                return Err(format!("example for {target} must be a number"));
            }
        },
        SchemaKind::String => {
            if !value.is_string() {
                return Err(format!("example for {target} must be a string"));
            }
        },
        SchemaKind::Null => {
            if !value.is_null() {
                return Err(format!("example for {target} must be null"));
            }
        },
        SchemaKind::Literal { value: expected } => {
            let actual = serde_json::to_value(value).map_err(|error| error.to_string())?;
            if &actual != expected {
                return Err(format!("example for {target} does not match its literal value"));
            }
        },
        SchemaKind::Enum { values } => {
            let actual = serde_json::to_value(value).map_err(|error| error.to_string())?;
            if !values.iter().any(|candidate| candidate == &actual) {
                return Err(format!("example for {target} is not an allowed enum value"));
            }
        },
    }
    Ok(())
}

fn validate_overrides(table: &[SemanticOverride], matched: &BTreeSet<usize>) -> Result<(), String> {
    let expected: BTreeSet<usize> = (0..table.len()).collect();
    if matched != &expected {
        let stale = expected
            .difference(matched)
            .map(|index| {
                format!(
                    "{}.{} ({})",
                    table[*index].owner, table[*index].field, table[*index].reason
                )
            })
            .collect::<Vec<_>>();
        return Err(format!(
            "stale or unmatched catalog semantic overrides: {}",
            stale.join(", ")
        ));
    }
    for (index, item) in table.iter().enumerate() {
        if table.iter().enumerate().any(|(other, candidate)| {
            other != index
                && candidate.owner == item.owner
                && candidate.field == item.field
                && candidate.rust_type == item.rust_type
        }) {
            return Err(format!(
                "ambiguous catalog semantic override: {}.{}",
                item.owner, item.field
            ));
        }
    }
    Ok(())
}

fn source_line(path: Option<&str>) -> Option<u32> {
    let source = path.and_then(|path| fs::read_to_string(path).ok())?;
    source.lines().enumerate().find_map(|(index, line)| {
        line.contains("fn name(")
            .then(|| u32::try_from(index.saturating_add(1)).ok())
            .flatten()
    })
}

struct SemanticOverride {
    owner: &'static str,
    field: &'static str,
    rust_type: &'static str,
    kind: &'static str,
    reason: &'static str,
}

const SEMANTIC_OVERRIDES: &[SemanticOverride] = &[
    SemanticOverride {
        owner: "core.root.config",
        field: "config",
        rust_type: "serde_yaml :: Value",
        kind: "filter_entry_config",
        reason: "FilterEntry dispatches config by its filter discriminator",
    },
    SemanticOverride {
        owner: "core.filter.http.observability.access_log",
        field: "fields",
        rust_type: "Option < Vec < serde_yaml :: Value > >",
        kind: "field_names",
        reason: "access-log field selectors are validated names",
    },
    SemanticOverride {
        owner: "core.filter.http.transformation.url_rewrite",
        field: "regex_replace",
        rust_type: "Option < serde_yaml :: Value >",
        kind: "query_operation",
        reason: "URL rewrite accepts an object or sequence operation",
    },
    SemanticOverride {
        owner: "core.filter.http.transformation.url_rewrite",
        field: "strip_query_params",
        rust_type: "Option < serde_yaml :: Value >",
        kind: "query_operation",
        reason: "URL rewrite accepts an object or sequence operation",
    },
    SemanticOverride {
        owner: "core.filter.http.transformation.url_rewrite",
        field: "add_query_params",
        rust_type: "Option < serde_yaml :: Value >",
        kind: "query_operation",
        reason: "URL rewrite accepts an object or sequence operation",
    },
    SemanticOverride {
        owner: "core.filter.http.security.guardrails",
        field: "contains",
        rust_type: "Option < ContainsValue >",
        kind: "contains_value",
        reason: "guardrail content target is one of the documented semantic values",
    },
];


struct CoreSchemaPolicy<'a> {
    root: &'a Path,
    owner: String,
    matched: RefCell<BTreeSet<usize>>,
}

impl<'a> CoreSchemaPolicy<'a> {
    fn new(root: &'a Path, owner: &str) -> Self {
        Self {
            root,
            owner: owner.to_owned(),
            matched: RefCell::new(BTreeSet::new()),
        }
    }
}

impl SchemaPolicy for CoreSchemaPolicy<'_> {
    fn field_schema(&self, field: &praxis_config_catalog_generator::SourceField) -> Result<Option<SchemaNode>, String> {
        let ty = &field.ty;
        let field_type = quote::quote!(#ty).to_string();
        let matches: Vec<(usize, &SemanticOverride)> = SEMANTIC_OVERRIDES
            .iter()
            .enumerate()
            .filter(|(_, item)| item.owner == self.owner && item.field == field.name && item.rust_type == field_type)
            .collect();
        if matches.len() > 1 {
            return Err(format!(
                "ambiguous semantic overrides for {}.{}",
                self.owner, field.name
            ));
        }
        if let Some((index, item)) = matches.into_iter().next() {
            self.matched.borrow_mut().insert(index);
            return Ok(Some(match item.kind {
                "filter_entry_config" => SchemaNode::reference("core.filter.entry.config".to_owned()),
                "field_names" => SchemaNode::array(SchemaNode::simple(SchemaKind::String)),
                "query_operation" => SchemaNode::one_of(vec![
                    SchemaNode::simple(SchemaKind::String),
                    SchemaNode::map(SchemaNode::simple(SchemaKind::String)),
                    SchemaNode::array(SchemaNode::simple(SchemaKind::String)),
                ]),
                "contains_value" => SchemaNode::one_of(vec![
                    SchemaNode::simple(SchemaKind::String),
                    SchemaNode::array(SchemaNode::simple(SchemaKind::String)),
                ]),
                _ => {
                    return Err(format!(
                        "unsupported semantic override kind for {}.{}: {}",
                        self.owner, field.name, item.kind
                    ));
                },
            }));
        }
        // FilterEntry is reused inside nested filter-bearing configs (for
        // example iterative request-router steps). Those fields have the same
        // dynamic payload contract as the root Config filter entries even
        // though their schema owner is not `core.root.config`.
        if field.name == "config" && field_type == "serde_yaml :: Value" {
            return Ok(Some(SchemaNode::reference("core.filter.entry.config".to_owned())));
        }
        Ok(custom_deserializer_kind_path(field.deserialize_with.as_deref()).map(|kind| SchemaNode::simple(kind)))
    }

    fn named_type_schema(&self, name: &str) -> Result<Option<SchemaNode>, String> {
        let node = match name {
            "Condition" => Some(condition_node(false)),
            "ResponseCondition" => Some(condition_node(true)),
            "HttpStatusCode" => Some(bounded_node(
                SchemaKind::Integer,
                numeric_rule("core.http_status_code.range", 100, 599),
            )),
            "RetryBodyLimit" => Some(bounded_node(
                SchemaKind::Integer,
                numeric_rule("core.retry_body_limit.range", 0, 65_536),
            )),
            "BudgetPercent" => Some(bounded_number_node(
                "core.budget_percent.range",
                serde_json::Value::from(0.0_f64),
                serde_json::Value::from(100.0_f64),
            )),
            "MaxEntries" => Some(bounded_node(
                SchemaKind::Integer,
                numeric_rule("core.max_entries.range", 1, 200_000),
            )),
            "Value" => return Err(format!("unresolved dynamic config type `Value` at {}", self.owner)),
            _ => None,
        };
        Ok(node)
    }

    fn default_value(&self, path: &str) -> Option<serde_json::Value> {
        evaluate_default(self.root, path)
    }

    fn schema_prefix(&self) -> &str {
        "core.type"
    }
}

fn numeric_rule(code: &str, min: i64, max: i64) -> praxis_config_catalog::PortableRule {
    let mut parameters = BTreeMap::new();
    parameters.insert("min".to_owned(), serde_json::Value::from(min));
    parameters.insert("max".to_owned(), serde_json::Value::from(max));
    praxis_config_catalog::PortableRule {
        code: code.to_owned(),
        target: String::new(),
        kind: praxis_config_catalog::RuleKind::NumericBounds,
        parameters,
        message: format!("value must be between {min} and {max}"),
    }
}

fn bounded_node(kind: SchemaKind, rule: praxis_config_catalog::PortableRule) -> SchemaNode {
    let mut node = SchemaNode::simple(kind);
    node.rules.push(rule);
    node
}

fn bounded_number_node(code: &str, min: serde_json::Value, max: serde_json::Value) -> SchemaNode {
    let mut parameters = BTreeMap::new();
    parameters.insert("min".to_owned(), min);
    parameters.insert("max".to_owned(), max);
    let mut node = SchemaNode::simple(SchemaKind::Number);
    node.rules.push(praxis_config_catalog::PortableRule {
        code: code.to_owned(),
        target: String::new(),
        kind: praxis_config_catalog::RuleKind::NumericBounds,
        parameters,
        message: "value must be between 0 and 100".to_owned(),
    });
    node
}

fn evaluate_default(root: &Path, function: &str) -> Option<serde_json::Value> {
    let files = [
        root.join("crates/core/src"),
        root.join("crates/filter/src"),
        root.join("crates/tls/src"),
    ];
    for directory in files {
        for path in super::filter_docs::collect_rs_files(&directory) {
            let Ok(source) = fs::read_to_string(path) else { continue };
            let marker = format!("fn {function}");
            let Some(start) = source.find(&marker) else { continue };
            let tail = &source[start..];
            let body = &tail[..tail.find('}').unwrap_or(tail.len())];
            if body.contains("true") {
                return Some(serde_json::Value::Bool(true));
            }
            if body.contains("false") {
                return Some(serde_json::Value::Bool(false));
            }
            if let Some(value) = body.split('"').nth(1) {
                return Some(serde_json::Value::String(value.to_owned()));
            }
            if let Some(number) = body
                .split(|character: char| !character.is_ascii_digit() && character != '-')
                .find(|value| !value.is_empty() && *value != "-")
            {
                if let Ok(value) = number.parse::<i64>() {
                    return Some(serde_json::Value::from(value));
                }
            }
        }
    }
    None
}

fn condition_node(response: bool) -> SchemaNode {
    let match_fields = if response {
        vec![
            ObjectField {
                serialized_name: "status".into(),
                aliases: Vec::new(),
                schema: SchemaNode::array(SchemaNode::simple(SchemaKind::Integer)),
                required: false,
                flattened: false,
            },
            ObjectField {
                serialized_name: "headers".into(),
                aliases: Vec::new(),
                schema: SchemaNode::map(SchemaNode::simple(SchemaKind::String)),
                required: false,
                flattened: false,
            },
        ]
    } else {
        vec![
            ObjectField {
                serialized_name: "path".into(),
                aliases: Vec::new(),
                schema: SchemaNode::simple(SchemaKind::String),
                required: false,
                flattened: false,
            },
            ObjectField {
                serialized_name: "path_prefix".into(),
                aliases: Vec::new(),
                schema: SchemaNode::simple(SchemaKind::String),
                required: false,
                flattened: false,
            },
            ObjectField {
                serialized_name: "methods".into(),
                aliases: Vec::new(),
                schema: SchemaNode::array(SchemaNode::simple(SchemaKind::String)),
                required: false,
                flattened: false,
            },
            ObjectField {
                serialized_name: "headers".into(),
                aliases: Vec::new(),
                schema: SchemaNode::map(SchemaNode::simple(SchemaKind::String)),
                required: false,
                flattened: false,
            },
        ]
    };
    let match_schema = SchemaNode::object(match_fields);
    SchemaNode::one_of(vec![
        SchemaNode::object(vec![ObjectField {
            serialized_name: "when".into(),
            aliases: Vec::new(),
            schema: match_schema.clone(),
            required: true,
            flattened: false,
        }]),
        SchemaNode::object(vec![ObjectField {
            serialized_name: "unless".into(),
            aliases: Vec::new(),
            schema: match_schema,
            required: true,
            flattened: false,
        }]),
    ])
}

fn custom_deserializer_kind_path(path: Option<&str>) -> Option<SchemaKind> {
    match path {
        Some("deserialize_redirect_status") => Some(SchemaKind::Enum {
            values: [301, 302, 307, 308].into_iter().map(serde_json::Value::from).collect(),
        }),
        _ => None,
    }
}

fn filter_capabilities(
    entry: &super::filter_docs::FilterEntry,
    registry: &praxis_filter::FilterRegistry,
) -> praxis_config_catalog::FilterCapabilities {
    let source = entry
        .filter
        .source_path
        .as_deref()
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let is_http = entry.protocol == "http";
    praxis_config_catalog::FilterCapabilities {
        security_class: if registry.is_security_filter(&entry.filter.name) {
            SecurityClass::Security
        } else {
            SecurityClass::Standard
        },
        request_headers: is_http && source.contains("fn on_request("),
        request_body: is_http && source.contains("fn on_request_body("),
        response_headers: is_http && source.contains("fn on_response("),
        response_body: is_http && source.contains("fn on_response_body("),
        terminal: praxis_core::config::TERMINAL_FILTERS.contains(&entry.filter.name.as_str()),
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

fn enabled_features(_root: &Path) -> BTreeSet<&'static str> {
    [
        ("basic-auth-filter", cfg!(feature = "basic-auth-filter")),
        ("policy-engine", cfg!(feature = "policy-engine")),
        ("otel", cfg!(feature = "otel")),
    ]
    .into_iter()
    .filter_map(|(feature, enabled)| enabled.then_some(feature))
    .collect()
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
            .get(&"core.type.RetryPolicy".into())
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

    #[test]
    fn semantic_override_validation_rejects_stale_and_ambiguous_entries() {
        let stale = [SemanticOverride {
            owner: "test",
            field: "field",
            rust_type: "Value",
            kind: "test",
            reason: "fixture",
        }];
        assert!(
            validate_overrides(&stale, &BTreeSet::new()).is_err(),
            "unmatched overrides must fail closed"
        );
        let ambiguous = [
            SemanticOverride {
                owner: "test",
                field: "field",
                rust_type: "Value",
                kind: "test",
                reason: "fixture",
            },
            SemanticOverride {
                owner: "test",
                field: "field",
                rust_type: "Value",
                kind: "test",
                reason: "duplicate",
            },
        ];
        assert!(
            validate_overrides(&ambiguous, &BTreeSet::from([0, 1])).is_err(),
            "ambiguous overrides must fail closed"
        );
    }

    #[test]
    fn example_validation_rejects_wrong_scalar_and_unknown_field() {
        let fragment = CatalogFragment {
            format_version: CatalogFormatVersion { major: 1, minor: 0 },
            producer: praxis_config_catalog::ProducerInfo {
                component: praxis_config_catalog::ProducerComponent::Core,
                package: "test".to_owned(),
                version: "1.0.0".to_owned(),
                source_revision: None,
            },
            compatibility: praxis_config_catalog::CatalogCompatibility {
                requires_format_major: 1,
                requires_core: None,
            },
            feature_profile: Default::default(),
            schemas: BTreeMap::new(),
            roots: Vec::new(),
            filters: Vec::new(),
            diagnostics: Vec::new(),
        };
        let scalar = SchemaNode::simple(SchemaKind::Boolean);
        assert!(
            validate_yaml_node(
                &serde_yaml::Value::String("true".to_owned()),
                &scalar,
                &fragment,
                "test"
            )
            .is_err()
        );
        let object = SchemaNode::object(vec![ObjectField {
            serialized_name: "known".to_owned(),
            aliases: Vec::new(),
            schema: SchemaNode::simple(SchemaKind::String),
            required: false,
            flattened: false,
        }]);
        let value: serde_yaml::Value = serde_yaml::from_str("unknown: value").expect("fixture YAML");
        assert!(validate_yaml_node(&value, &object, &fragment, "test").is_err());
    }
}
