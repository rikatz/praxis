// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Stable, runtime-independent types used to describe Praxis configuration.
//!
//! This crate deliberately contains no proxy, filesystem, YAML, or async
//! dependencies so it can be consumed by browser/WASM configuration tools.

mod schema_trait;
pub mod std_impls;

use std::collections::{BTreeMap, BTreeSet};

pub use praxis_config_catalog_derive::ConfigSchemaFor;
// Re-export the trait following the pattern: pub use praxis_config_catalog::ConfigSchemaFor;
pub use schema_trait::ConfigSchemaFor;
use serde::{Deserialize, Serialize};

/// Current wire format version.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogFormatVersion {
    /// Incompatible changes increment the major version.
    pub major: u16,
    /// Additive changes increment the minor version.
    pub minor: u16,
}

/// Component producing a catalog fragment.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(
    missing_docs,
    reason = "serialized enum variants are defined by the catalog wire contract"
)]
pub enum ProducerComponent {
    Core,
    Ai,
    Extension,
}

/// Producer provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProducerInfo {
    /// Logical producer kind.
    pub component: ProducerComponent,
    /// Package name.
    pub package: String,
    /// Package version.
    pub version: String,
    /// Optional source revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
}

/// Compatibility requirements for a fragment.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogCompatibility {
    /// Required catalog format major version.
    pub requires_format_major: u16,
    /// Optional compatible Core package requirement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_core: Option<VersionRequirement>,
}

/// Features available when a fragment was generated.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FeatureProfile {
    /// Features enabled for this artifact.
    #[serde(default)]
    pub available: BTreeSet<String>,
    /// Features enabled by default.
    #[serde(default)]
    pub default_enabled: BTreeSet<String>,
}

/// Stable schema identifier.
#[derive(Clone, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SchemaId(pub String);

impl From<String> for SchemaId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SchemaId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Display for SchemaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A semver requirement for the Core package.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct VersionRequirement(pub String);

impl From<String> for VersionRequirement {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for VersionRequirement {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// A complete producer-owned catalog fragment.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CatalogFragment {
    /// Wire format version.
    pub format_version: CatalogFormatVersion,
    /// Producer provenance.
    pub producer: ProducerInfo,
    /// Compatibility requirements.
    pub compatibility: CatalogCompatibility,
    /// Feature profile used to generate this fragment.
    pub feature_profile: FeatureProfile,
    /// Named reusable schemas.
    #[serde(default)]
    pub schemas: BTreeMap<SchemaId, ConfigSchema>,
    /// Root configuration descriptors.
    #[serde(default)]
    pub roots: Vec<RootConfigDescriptor>,
    /// Filter descriptors.
    #[serde(default)]
    pub filters: Vec<FilterDescriptor>,
    /// Non-fatal generation diagnostics.
    #[serde(default)]
    pub diagnostics: Vec<CatalogDiagnostic>,
}

/// Named configuration schema.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConfigSchema {
    /// Stable identifier.
    pub id: SchemaId,
    /// Human-readable title.
    pub title: String,
    /// Documentation.
    #[serde(default)]
    pub description: String,
    /// Schema node.
    pub node: SchemaNode,
    /// Whether this schema is intentionally shared across producer fragments.
    ///
    /// Identical definitions may be coalesced during merge only when both
    /// producers explicitly opt in. Provenance is still retained on the
    /// resulting schema.
    #[serde(default, skip_serializing_if = "is_false")]
    pub shared: bool,
    /// Producer provenance, populated for merged catalogs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<ProducerInfo>,
}

/// Omit a false wire flag while retaining serde's default for old fragments.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires a reference predicate"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

/// Schema node for a configuration value.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SchemaNode {
    /// Node shape.
    pub kind: SchemaKind,
    /// Optional title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Documentation.
    #[serde(default)]
    pub description: String,
    /// Literal default value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Example values.
    #[serde(default)]
    pub examples: Vec<serde_json::Value>,
    /// Portable constraints.
    #[serde(default)]
    pub rules: Vec<PortableRule>,
    /// Whether the value contains credentials or other secrets.
    #[serde(default)]
    pub sensitive: bool,
}

impl SchemaNode {
    /// Construct a schema node with empty optional metadata.
    #[must_use]
    pub fn simple(kind: SchemaKind) -> Self {
        Self {
            kind,
            title: None,
            description: String::new(),
            default: None,
            examples: Vec::new(),
            rules: Vec::new(),
            sensitive: false,
        }
    }

    /// Construct an array schema node.
    #[must_use]
    pub fn array(items: SchemaNode) -> Self {
        Self::simple(SchemaKind::Array {
            items: Box::new(items),
            min_items: None,
            max_items: None,
        })
    }

    /// Construct a map schema node.
    #[must_use]
    pub fn map(values: SchemaNode) -> Self {
        Self::simple(SchemaKind::Map {
            values: Box::new(values),
        })
    }

    /// Construct an object schema node.
    #[must_use]
    pub fn object(fields: Vec<ObjectField>) -> Self {
        Self::simple(SchemaKind::Object {
            fields,
            additional_properties: false,
        })
    }

    /// Construct an enum of string values.
    #[must_use]
    pub fn enum_strings(values: Vec<String>) -> Self {
        Self::simple(SchemaKind::Enum {
            values: values.into_iter().map(serde_json::Value::String).collect(),
        })
    }

    /// Construct a literal schema node.
    #[must_use]
    pub fn literal(value: serde_json::Value) -> Self {
        Self::simple(SchemaKind::Literal { value })
    }

    /// Construct a one-of schema node.
    #[must_use]
    pub fn one_of(variants: Vec<SchemaNode>) -> Self {
        Self::one_of_with_discriminator(variants, None)
    }

    /// Construct a tagged union schema with an optional discriminator field.
    #[must_use]
    pub fn one_of_with_discriminator(variants: Vec<SchemaNode>, discriminator: Option<String>) -> Self {
        Self::simple(SchemaKind::OneOf {
            variants,
            discriminator,
        })
    }

    /// Construct a reference schema node.
    #[must_use]
    pub fn reference(schema_id: String) -> Self {
        Self::simple(SchemaKind::Reference {
            schema_id: SchemaId(schema_id),
        })
    }
}

/// Serializable configuration shape.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[expect(
    missing_docs,
    reason = "serialized schema variants are defined by the catalog wire contract"
)]
pub enum SchemaKind {
    /// Unresolved source types are retained only for diagnostics and always
    /// rejected by validation; generators must never emit this variant.
    Any,
    Boolean,
    Integer,
    Number,
    String,
    Null,
    Literal {
        value: serde_json::Value,
    },
    Enum {
        values: Vec<serde_json::Value>,
    },
    Object {
        fields: Vec<ObjectField>,
        additional_properties: bool,
    },
    Array {
        items: Box<SchemaNode>,
        min_items: Option<usize>,
        max_items: Option<usize>,
    },
    Map {
        values: Box<SchemaNode>,
    },
    OneOf {
        variants: Vec<SchemaNode>,
        discriminator: Option<String>,
    },
    Reference {
        schema_id: SchemaId,
    },
}

/// Field in an object schema.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[expect(missing_docs, reason = "serialized fields are defined by the catalog wire contract")]
pub struct ObjectField {
    pub serialized_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub schema: SchemaNode,
    pub required: bool,
    #[serde(default)]
    pub flattened: bool,
}

/// A filter's configuration descriptor.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[expect(missing_docs, reason = "serialized fields are defined by the catalog wire contract")]
pub struct FilterDescriptor {
    pub name: String,
    pub protocol: Protocol,
    pub category: String,
    #[serde(default)]
    pub description: String,
    pub config_schema: SchemaId,
    #[serde(default)]
    pub required_features: BTreeSet<String>,
    pub capabilities: FilterCapabilities,
    #[serde(default)]
    pub examples: Vec<ConfigExample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
    /// Producer provenance retained when fragments are merged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<ProducerInfo>,
}

/// Runtime phase capabilities of a filter.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[expect(missing_docs, reason = "serialized fields are defined by the catalog wire contract")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "phase capability flags are an independent wire contract"
)]
pub struct FilterCapabilities {
    #[serde(default)]
    pub security_class: SecurityClass,
    #[serde(default)]
    pub request_headers: bool,
    #[serde(default)]
    pub request_body: bool,
    #[serde(default)]
    pub response_headers: bool,
    #[serde(default)]
    pub response_body: bool,
    #[serde(default)]
    pub terminal: bool,
}

/// Example YAML configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[expect(missing_docs, reason = "serialized fields are defined by the catalog wire contract")]
pub struct ConfigExample {
    pub yaml: String,
}

/// Source location relative to the producer repository.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[expect(missing_docs, reason = "serialized fields are defined by the catalog wire contract")]
pub struct SourceLocation {
    pub path: String,
    pub line: Option<u32>,
}

/// Root configuration descriptor.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[expect(missing_docs, reason = "serialized fields are defined by the catalog wire contract")]
pub struct RootConfigDescriptor {
    pub name: String,
    pub schema: SchemaId,
}

/// A portable validation rule.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[expect(missing_docs, reason = "serialized fields are defined by the catalog wire contract")]
pub struct PortableRule {
    pub code: String,
    pub target: String,
    pub kind: RuleKind,
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_json::Value>,
    pub message: String,
}

/// A generator diagnostic.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[expect(missing_docs, reason = "serialized fields are defined by the catalog wire contract")]
pub struct CatalogDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub target: String,
    pub message: String,
}

/// Protocol implemented by a filter.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(missing_docs, reason = "variants are defined by the catalog wire contract")]
pub enum Protocol {
    Http,
    Tcp,
}

/// Security classification for a filter.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(missing_docs, reason = "variants are defined by the catalog wire contract")]
pub enum SecurityClass {
    #[default]
    Standard,
    Security,
}

/// Severity of a generation diagnostic.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(missing_docs, reason = "variants are defined by the catalog wire contract")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// Portable validation rule kinds supported by format version 1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(missing_docs, reason = "variants are defined by the catalog wire contract")]
pub enum RuleKind {
    NumericBounds,
    LengthBounds,
    Pattern,
    NonEmpty,
    MutualExclusion,
    AtLeastOne,
    UniqueItems,
    ReferenceToNamedObject,
}

impl CatalogFragment {
    /// Validate the complete fragment contract.
    ///
    /// # Errors
    ///
    /// Returns a structured error when any semantic identity, compatibility
    /// requirement, or schema reference violates the wire contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.format_version.major != self.compatibility.requires_format_major {
            return Err(ValidationError::Compatibility {
                expected: self.format_version.major,
                found: self.compatibility.requires_format_major,
            });
        }
        self.validate_descriptors()?;
        if !self
            .feature_profile
            .default_enabled
            .is_subset(&self.feature_profile.available)
        {
            return Err(ValidationError::UnknownFeature {
                filter: "feature_profile.default_enabled".to_owned(),
            });
        }
        if let Some(filter) = self
            .filters
            .iter()
            .find(|filter| !filter.required_features.is_subset(&self.feature_profile.available))
        {
            return Err(ValidationError::UnknownFeature {
                filter: filter.name.clone(),
            });
        }
        self.validate_schema_ids()?;
        self.validate_schema_nodes()?;
        Ok(())
    }

    /// Validate descriptor identity and references.
    fn validate_descriptors(&self) -> Result<(), ValidationError> {
        let mut names = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for filter in &self.filters {
            if !names.insert(&filter.name) {
                return Err(ValidationError::DuplicateFilter(filter.name.clone()));
            }
            if let Some(source) = &filter.source
                && !sources.insert((&filter.name, &source.path, source.line))
            {
                return Err(ValidationError::DuplicateSource(source.path.clone()));
            }
            if !self.schemas.contains_key(&filter.config_schema) {
                return Err(ValidationError::DanglingSchema {
                    target: filter.config_schema.clone(),
                });
            }
        }
        let mut roots = BTreeSet::new();
        for root in &self.roots {
            if !roots.insert(&root.name) {
                return Err(ValidationError::Duplicate {
                    kind: "root",
                    name: root.name.clone(),
                });
            }
            if !self.schemas.contains_key(&root.schema) {
                return Err(ValidationError::DanglingSchema {
                    target: root.schema.clone(),
                });
            }
        }
        Ok(())
    }

    /// Validate map keys against embedded schema IDs.
    fn validate_schema_ids(&self) -> Result<(), ValidationError> {
        for (id, schema) in &self.schemas {
            if id != &schema.id {
                return Err(ValidationError::SchemaIdMismatch {
                    key: id.clone(),
                    embedded: schema.id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Validate every schema node recursively.
    fn validate_schema_nodes(&self) -> Result<(), ValidationError> {
        for (id, schema) in &self.schemas {
            validate_node(&schema.node, &self.schemas).map_err(|error| ValidationError::Schema {
                id: id.clone(),
                source: Box::new(error),
            })?;
        }
        Ok(())
    }
}

/// Structured errors returned by fragment validation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[expect(missing_docs, reason = "variants describe stable structured generator failures")]
pub enum ValidationError {
    Compatibility { expected: u16, found: u16 },
    Duplicate { kind: &'static str, name: String },
    DanglingSchema { target: SchemaId },
    SchemaIdMismatch { key: SchemaId, embedded: SchemaId },
    UnresolvedAny,
    Schema { id: SchemaId, source: Box<Self> },
    MissingFeatures { filter: String },
    DuplicateFilter(String),
    DuplicateSource(String),
    UnknownFeature { filter: String },
}

impl ValidationError {
    /// Stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Compatibility { .. } => "format_major_mismatch",
            Self::Duplicate { .. } => "duplicate_identity",
            Self::DuplicateFilter(_) => "duplicate_filter",
            Self::DuplicateSource(_) => "duplicate_source",
            Self::DanglingSchema { .. } => "dangling_reference",
            Self::SchemaIdMismatch { .. } => "schema_id_mismatch",
            Self::UnresolvedAny => "unresolved_any",
            Self::Schema { source, .. } => source.code(),
            Self::MissingFeatures { .. } => "missing_features",
            Self::UnknownFeature { .. } => "unknown_feature",
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compatibility { expected, found } => {
                write!(f, "format major {found} is incompatible with {expected}")
            },
            Self::Duplicate { kind, name } => write!(f, "duplicate {kind}: {name}"),
            Self::DanglingSchema { target } => write!(f, "dangling schema: {target}"),
            Self::SchemaIdMismatch { key, embedded } => {
                write!(f, "schema key {key} does not match embedded id {embedded}")
            },
            Self::UnresolvedAny => f.write_str("unresolved `any` schema is not allowed"),
            Self::DuplicateFilter(name) => write!(f, "duplicate filter: {name}"),
            Self::DuplicateSource(path) => write!(f, "duplicate filter source: {path}"),
            Self::MissingFeatures { filter } | Self::UnknownFeature { filter } => {
                write!(f, "filter {filter} requires unavailable features")
            },
            Self::Schema { id, source } => write!(f, "{id}: {source}"),
        }
    }
}

/// Validate a schema node and every nested reference.
fn validate_node(node: &SchemaNode, schemas: &BTreeMap<SchemaId, ConfigSchema>) -> Result<(), ValidationError> {
    match &node.kind {
        SchemaKind::Any => Err(ValidationError::UnresolvedAny),
        SchemaKind::Reference { schema_id } if !schemas.contains_key(schema_id) => {
            Err(ValidationError::DanglingSchema {
                target: schema_id.clone(),
            })
        },
        SchemaKind::Object { fields, .. } => fields
            .iter()
            .try_for_each(|field| validate_node(&field.schema, schemas)),
        SchemaKind::Array { items, .. } => validate_node(items, schemas),
        SchemaKind::Map { values } => validate_node(values, schemas),
        SchemaKind::OneOf { variants, .. } => variants.iter().try_for_each(|variant| validate_node(variant, schemas)),
        SchemaKind::Boolean
        | SchemaKind::Integer
        | SchemaKind::Number
        | SchemaKind::String
        | SchemaKind::Null
        | SchemaKind::Literal { .. }
        | SchemaKind::Enum { .. }
        | SchemaKind::Reference { .. } => Ok(()),
    }
}
