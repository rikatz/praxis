//! Build-time helpers for producing Praxis configuration catalog artifacts.
//!
//! This crate is deliberately separate from `praxis-config-catalog`: the
//! latter remains safe for runtime and WASM consumers, while this crate is the
//! development-time seam used by catalog producers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

pub use praxis_config_catalog::generator::{GenerationError, MergeError};
use praxis_config_catalog::{
    CatalogDiagnostic, CatalogFragment, ConfigSchema, MergedCatalog, ObjectField, SchemaId, SchemaKind, SchemaNode,
};

/// Producer-neutral Rust syntax and serde metadata extraction.
pub mod source;

pub use source::{
    RustEnum, RustEnumVariant, RustEnumVariantShape, RustField, RustSourceModel, RustStruct, SourceParseError,
    collect_rust_files, parse_rust_file, parse_rust_files, parse_rust_source,
};

/// A normalized source field understood by the catalog schema builder.
///
/// Producer adapters are responsible for extracting this representation from
/// Rust source. The builder then applies the same serde-facing rules to every
/// producer.
#[derive(Clone, Debug)]
pub struct SourceField {
    /// Serialized field name.
    pub name: String,
    /// Serde aliases accepted on input.
    pub aliases: Vec<String>,
    /// Rust type of the field.
    pub ty: syn::Type,
    /// Field documentation.
    pub doc: String,
    /// Whether serde supplies a default for this field.
    pub has_default: bool,
    /// Optional path to the default function, when one was declared.
    pub default_path: Option<String>,
    /// Optional serde deserializer responsible for the wire representation.
    ///
    /// The producer policy may use this to replace the Rust type's default
    /// schema when a custom deserializer changes the accepted input shape.
    pub deserialize_with: Option<String>,
    /// Whether this field is flattened by serde.
    pub flatten: bool,
    /// Whether the field is required independently of its Rust type.
    pub requirement: RequirementHint,
    /// Explicit sensitivity metadata from the source adapter.
    pub sensitive: bool,
}

/// Source-level requiredness hint.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequirementHint {
    /// The field is normally required unless it has a default or is optional.
    #[default]
    Normal,
    /// The source adapter knows that the field is not required.
    Optional,
}

/// A normalized Rust struct declaration.
#[derive(Clone, Debug)]
pub struct SourceStruct {
    /// Rust type name.
    pub name: String,
    /// Documentation for the type.
    pub doc: String,
    /// Declared fields in source order.
    pub fields: Vec<SourceField>,
    /// Unsupported conversion metadata, retained so adapters can fail loudly.
    pub try_from: Option<String>,
}

/// Shape of a Rust enum variant.
#[derive(Clone, Debug)]
pub enum SourceEnumVariantShape {
    /// A unit variant.
    Unit,
    /// A tuple variant with one or more values.
    Unnamed(Vec<syn::Type>),
    /// A struct-like variant.
    Named(Vec<SourceField>),
}

/// A normalized Rust enum variant.
#[derive(Clone, Debug)]
pub struct SourceEnumVariant {
    /// Rust variant name and default serialized label.
    pub name: String,
    /// Variant payload shape.
    pub shape: SourceEnumVariantShape,
}

/// A normalized Rust enum declaration and its serde tagging policy.
#[derive(Clone, Debug)]
pub struct SourceEnum {
    /// Rust type name.
    pub name: String,
    /// Documentation for the type.
    pub doc: String,
    /// Declared variants in source order.
    pub variants: Vec<SourceEnumVariant>,
    /// Adjacent or internally tagged discriminator name.
    pub tag: Option<String>,
    /// Adjacent-tag content name.
    pub content: Option<String>,
    /// Whether serde treats this enum as untagged.
    pub untagged: bool,
}

/// Source declarations used by [`build_fields`].
#[derive(Clone, Debug, Default)]
pub struct SourceModel {
    /// Struct declarations keyed by Rust type name.
    pub structs: BTreeMap<String, SourceStruct>,
    /// Enum declarations keyed by Rust type name.
    pub enums: BTreeMap<String, SourceEnum>,
}

impl RustSourceModel {
    /// Convert parsed Rust declarations into the normalized schema-builder
    /// model without applying any producer-specific policy.
    pub fn schema_model(&self) -> SourceModel {
        SourceModel {
            structs: self
                .structs
                .iter()
                .map(|(name, source)| {
                    (
                        name.clone(),
                        SourceStruct {
                            name: source.name.clone(),
                            doc: source.docs.clone(),
                            fields: Self::schema_fields(&source.fields),
                            try_from: source.try_from.clone(),
                        },
                    )
                })
                .collect(),
            enums: self
                .enums
                .iter()
                .map(|(name, source)| {
                    let variants = source
                        .variants
                        .iter()
                        .map(|variant| SourceEnumVariant {
                            name: variant.name.clone(),
                            shape: match &variant.shape {
                                RustEnumVariantShape::Unit => SourceEnumVariantShape::Unit,
                                RustEnumVariantShape::Unnamed(types) => SourceEnumVariantShape::Unnamed(types.clone()),
                                RustEnumVariantShape::Named(fields) => {
                                    SourceEnumVariantShape::Named(Self::schema_fields(fields))
                                },
                            },
                        })
                        .collect();
                    (
                        name.clone(),
                        SourceEnum {
                            name: source.name.clone(),
                            doc: source.docs.clone(),
                            variants,
                            tag: source.tag.clone(),
                            content: source.content.clone(),
                            untagged: source.untagged,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Convert selected parsed fields for a root or filter schema.
    pub fn schema_fields(fields: &[RustField]) -> Vec<SourceField> {
        fields
            .iter()
            .filter(|field| !field.skip)
            .map(|field| SourceField {
                name: field.name.clone(),
                aliases: field.aliases.clone(),
                ty: field.ty.clone(),
                doc: field.docs.clone(),
                has_default: field.has_default,
                default_path: field.default_path.clone(),
                deserialize_with: field.deserialize_with.clone(),
                flatten: field.flatten,
                requirement: RequirementHint::Normal,
                sensitive: false,
            })
            .collect()
    }
}

/// Producer-specific hooks at the source-to-schema seam.
pub trait SchemaPolicy {
    /// Override the complete schema for a field, for custom deserializers.
    fn field_schema(&self, _field: &SourceField) -> Result<Option<SchemaNode>, String> {
        Ok(None)
    }

    /// Override a named Rust type that is not represented by [`SourceModel`].
    fn named_type_schema(&self, _name: &str) -> Result<Option<SchemaNode>, String> {
        Ok(None)
    }

    /// Evaluate a declared default without executing producer code.
    fn default_value(&self, _path: &str) -> Option<serde_json::Value> {
        None
    }

    /// Prefix used for generated reusable schema IDs.
    fn schema_prefix(&self) -> &str {
        "type"
    }

    /// Whether a field contains a secret. The default combines explicit source
    /// metadata with the conventional `secret` type-name marker.
    fn is_sensitive(&self, field: &SourceField) -> bool {
        field.sensitive || type_text(&field.ty).to_ascii_lowercase().contains("secret")
    }

    /// Build the stable ID for a source type.
    fn schema_id(&self, name: &str) -> SchemaId {
        SchemaId::from(format!("{}.{}", self.schema_prefix(), name))
    }
}

/// Policy with the portable default behavior.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultSchemaPolicy;

impl SchemaPolicy for DefaultSchemaPolicy {}

/// Error produced while converting normalized source declarations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaBuildError {
    /// Configuration target being converted.
    pub target: String,
    /// Human-readable failure reason.
    pub message: String,
}

impl std::fmt::Display for SchemaBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.target, self.message)
    }
}

impl std::error::Error for SchemaBuildError {}

/// Build object fields from normalized declarations and populate referenced
/// reusable schemas in `schemas`.
pub fn build_fields(
    fields: &[SourceField],
    model: &SourceModel,
    schemas: &mut BTreeMap<SchemaId, ConfigSchema>,
    diagnostics: &mut Vec<CatalogDiagnostic>,
    owner: &str,
    policy: &dyn SchemaPolicy,
) -> Result<Vec<ObjectField>, SchemaBuildError> {
    let mut builder = SchemaBuilder {
        model,
        schemas,
        diagnostics,
        owner: owner.to_owned(),
        policy,
        visiting: BTreeSet::new(),
    };
    builder.fields(fields)
}

struct SchemaBuilder<'a> {
    model: &'a SourceModel,
    schemas: &'a mut BTreeMap<SchemaId, ConfigSchema>,
    diagnostics: &'a mut Vec<CatalogDiagnostic>,
    owner: String,
    policy: &'a dyn SchemaPolicy,
    visiting: BTreeSet<String>,
}

impl SchemaBuilder<'_> {
    fn fields(&mut self, fields: &[SourceField]) -> Result<Vec<ObjectField>, SchemaBuildError> {
        fields
            .iter()
            .map(|field| {
                let schema = self.field_node(field)?;
                Ok(ObjectField {
                    serialized_name: field.name.clone(),
                    aliases: field.aliases.clone(),
                    schema,
                    required: matches!(field.requirement, RequirementHint::Normal)
                        && !field.has_default
                        && !field.flatten
                        && !is_option_type(&field.ty),
                    flattened: field.flatten,
                })
            })
            .collect()
    }

    fn field_node(&mut self, field: &SourceField) -> Result<SchemaNode, SchemaBuildError> {
        let mut node = if let Some(node) = self
            .policy
            .field_schema(field)
            .map_err(|message| self.error(field.name.clone(), message))?
        {
            node
        } else {
            self.type_node(&field.ty, &field.name)?
        };
        node.description = field.doc.clone();
        node.sensitive = self.policy.is_sensitive(field);
        if let Some(path) = field.default_path.as_deref() {
            if let Some(value) = self.policy.default_value(path) {
                node.default = Some(value);
            } else {
                self.diagnostics.push(CatalogDiagnostic {
                    severity: praxis_config_catalog::DiagnosticSeverity::Warning,
                    code: "unevaluable_default".to_owned(),
                    target: format!("{}.{}", self.owner, field.name),
                    message: format!("default function `{path}` could not be evaluated safely"),
                });
            }
        }
        Ok(node)
    }

    fn type_node(&mut self, ty: &syn::Type, target: &str) -> Result<SchemaNode, SchemaBuildError> {
        match ty {
            syn::Type::Reference(reference) => self.type_node(&reference.elem, target),
            syn::Type::Array(array) => Ok(SchemaNode::array(self.type_node(&array.elem, target)?)),
            syn::Type::Slice(slice) => Ok(SchemaNode::array(self.type_node(&slice.elem, target)?)),
            syn::Type::Tuple(tuple) if tuple.elems.is_empty() => Ok(SchemaNode::simple(SchemaKind::Null)),
            syn::Type::Tuple(_) => {
                Err(self.error(target.to_owned(), format!("unsupported tuple type `{}`", type_text(ty))))
            },
            syn::Type::Path(path) => self.path_node(path, target),
            _ => Err(self.error(target.to_owned(), format!("unsupported Rust type `{}`", type_text(ty)))),
        }
    }

    fn path_node(&mut self, path: &syn::TypePath, target: &str) -> Result<SchemaNode, SchemaBuildError> {
        let segment = path
            .path
            .segments
            .last()
            .ok_or_else(|| self.error(target.to_owned(), "empty type path"))?;
        let name = segment.ident.to_string();
        let args = type_args(&segment.arguments);
        match name.as_str() {
            "Option" | "Box" | "Arc" | "Rc" | "Cow" | "Zeroizing" => args
                .last()
                .ok_or_else(|| self.error(target.to_owned(), format!("missing inner type for `{name}`")))
                .and_then(|inner| self.type_node(inner, target)),
            "Vec" | "VecDeque" | "SmallVec" => {
                let inner = args
                    .last()
                    .ok_or_else(|| self.error(target.to_owned(), format!("missing item type for `{name}`")))?;
                let inner = if name == "SmallVec" {
                    match inner {
                        syn::Type::Array(array) => &*array.elem,
                        _ => inner,
                    }
                } else {
                    inner
                };
                Ok(SchemaNode::array(self.type_node(inner, target)?))
            },
            "BTreeMap" | "HashMap" | "IndexMap" => {
                let key = args
                    .first()
                    .ok_or_else(|| self.error(target.to_owned(), format!("missing map key for `{name}`")))?;
                let key_name = type_text(key);
                if key_name != "String" && key_name != "str" {
                    return Err(self.error(target.to_owned(), format!("non-string map key `{key_name}`")));
                }
                let value = args
                    .get(1)
                    .ok_or_else(|| self.error(target.to_owned(), format!("missing map value for `{name}`")))?;
                Ok(SchemaNode::map(self.type_node(value, target)?))
            },
            "bool" => Ok(SchemaNode::simple(SchemaKind::Boolean)),
            "f32" | "f64" => Ok(SchemaNode::simple(SchemaKind::Number)),
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => {
                Ok(SchemaNode::simple(SchemaKind::Integer))
            },
            "String" | "str" | "PathBuf" | "Url" | "Uri" | "HeaderName" | "HeaderValue" | "IpAddr" | "Ipv4Addr"
            | "Ipv6Addr" | "SocketAddr" | "SocketAddrV4" | "SocketAddrV6" | "Duration" | "Regex" | "SecretString" => {
                Ok(SchemaNode::simple(SchemaKind::String))
            },
            _ => {
                if let Some(node) = self
                    .policy
                    .named_type_schema(&name)
                    .map_err(|message| self.error(target.to_owned(), message))?
                {
                    return Ok(node);
                }
                if self.model.structs.contains_key(&name) {
                    self.struct_node(&name)
                } else if self.model.enums.contains_key(&name) {
                    self.enum_node(&name)
                } else {
                    Err(self.error(target.to_owned(), format!("unresolved configuration type `{name}`")))
                }
            },
        }
    }

    fn struct_node(&mut self, name: &str) -> Result<SchemaNode, SchemaBuildError> {
        let id = self.policy.schema_id(name);
        if self.schemas.contains_key(&id) || self.visiting.contains(name) {
            return Ok(SchemaNode::reference(id.0));
        }
        let source = self
            .model
            .structs
            .get(name)
            .ok_or_else(|| self.error(name.to_owned(), format!("missing struct `{name}`")))?;
        if let Some(try_from) = source.try_from.as_deref() {
            return Err(self.error(name.to_owned(), format!("unsupported TryFrom conversion `{try_from}`")));
        }
        self.visiting.insert(name.to_owned());
        let fields = self.fields(&source.fields)?;
        self.visiting.remove(name);
        self.schemas.insert(
            id.clone(),
            ConfigSchema {
                id: id.clone(),
                title: name.to_owned(),
                description: source.doc.clone(),
                node: SchemaNode::object(fields),
                shared: false,
                producer: None,
            },
        );
        Ok(SchemaNode::reference(id.0))
    }

    fn enum_node(&mut self, name: &str) -> Result<SchemaNode, SchemaBuildError> {
        let id = self.policy.schema_id(name);
        if self.schemas.contains_key(&id) || self.visiting.contains(name) {
            return Ok(SchemaNode::reference(id.0));
        }
        let source = self
            .model
            .enums
            .get(name)
            .ok_or_else(|| self.error(name.to_owned(), format!("missing enum `{name}`")))?
            .clone();
        self.visiting.insert(name.to_owned());
        let unit_only = !source.untagged
            && source.tag.is_none()
            && source
                .variants
                .iter()
                .all(|variant| matches!(variant.shape, SourceEnumVariantShape::Unit));
        let node = if unit_only {
            SchemaNode::enum_strings(source.variants.iter().map(|variant| variant.name.clone()).collect())
        } else {
            let variants = source
                .variants
                .iter()
                .map(|variant| self.variant_node(&source, variant))
                .collect::<Result<Vec<_>, _>>()?;
            SchemaNode::one_of_with_discriminator(variants, source.tag.clone())
        };
        self.visiting.remove(name);
        self.schemas.insert(
            id.clone(),
            ConfigSchema {
                id: id.clone(),
                title: name.to_owned(),
                description: source.doc,
                node,
                shared: false,
                producer: None,
            },
        );
        Ok(SchemaNode::reference(id.0))
    }

    fn variant_node(
        &mut self,
        source: &SourceEnum,
        variant: &SourceEnumVariant,
    ) -> Result<SchemaNode, SchemaBuildError> {
        match &variant.shape {
            SourceEnumVariantShape::Unit if source.tag.is_none() => {
                Ok(SchemaNode::literal(serde_json::Value::String(variant.name.clone())))
            },
            SourceEnumVariantShape::Unit => self.tagged_variant(source, &variant.name, Vec::new()),
            SourceEnumVariantShape::Unnamed(types) if source.content.is_some() => {
                let ty = types
                    .first()
                    .ok_or_else(|| self.error(variant.name.clone(), "empty tuple variant"))?;
                let field = ObjectField {
                    serialized_name: source.content.clone().unwrap_or_default(),
                    aliases: Vec::new(),
                    schema: self.type_node(ty, &variant.name)?,
                    required: true,
                    flattened: false,
                };
                self.tagged_variant(source, &variant.name, vec![field])
            },
            SourceEnumVariantShape::Unnamed(types) => {
                let ty = types
                    .first()
                    .ok_or_else(|| self.error(variant.name.clone(), "empty tuple variant"))?;
                self.type_node(ty, &variant.name)
            },
            SourceEnumVariantShape::Named(fields) => {
                let fields = self.fields(fields)?;
                self.tagged_variant(source, &variant.name, fields)
            },
        }
    }

    fn tagged_variant(
        &self,
        source: &SourceEnum,
        name: &str,
        mut fields: Vec<ObjectField>,
    ) -> Result<SchemaNode, SchemaBuildError> {
        let Some(tag) = source.tag.as_ref() else {
            return Ok(SchemaNode::literal(serde_json::Value::String(name.to_owned())));
        };
        fields.insert(
            0,
            ObjectField {
                serialized_name: tag.clone(),
                aliases: Vec::new(),
                schema: SchemaNode::literal(serde_json::Value::String(name.to_owned())),
                required: true,
                flattened: false,
            },
        );
        Ok(SchemaNode::object(fields))
    }

    fn error(&self, target: String, message: impl Into<String>) -> SchemaBuildError {
        SchemaBuildError {
            target: format!("{}.{}", self.owner, target),
            message: message.into(),
        }
    }
}

fn type_args(arguments: &syn::PathArguments) -> Vec<&syn::Type> {
    let syn::PathArguments::AngleBracketed(arguments) = arguments else {
        return Vec::new();
    };
    arguments
        .args
        .iter()
        .filter_map(|argument| {
            if let syn::GenericArgument::Type(ty) = argument {
                Some(ty)
            } else {
                None
            }
        })
        .collect()
}

fn type_text(ty: &syn::Type) -> String {
    quote::quote!(#ty).to_string().replace(" :: ", "::")
}

fn is_option_type(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else { return false };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Option")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestPolicy;

    impl SchemaPolicy for TestPolicy {
        fn schema_prefix(&self) -> &str {
            "test.type"
        }

        fn default_value(&self, path: &str) -> Option<serde_json::Value> {
            (path == "defaults::port").then(|| serde_json::json!(8080))
        }
    }

    fn field(name: &str, ty: &str) -> SourceField {
        SourceField {
            name: name.to_owned(),
            aliases: Vec::new(),
            ty: syn::parse_str(ty).expect("test type parses"),
            doc: String::new(),
            has_default: false,
            default_path: None,
            deserialize_with: None,
            flatten: false,
            requirement: RequirementHint::Normal,
            sensitive: false,
        }
    }

    #[test]
    fn builds_primitives_collections_and_field_metadata() {
        let mut port = field("port", "u16");
        port.doc = "Listen port".to_owned();
        port.default_path = Some("defaults::port".to_owned());
        let mut token = field("token", "SecretString");
        token.has_default = true;
        let mut optional = field("optional", "Option<String>");
        optional.flatten = true;
        let fields = vec![
            port,
            token,
            optional,
            field("tags", "Vec<String>"),
            field("labels", "BTreeMap<String, String>"),
        ];
        let mut schemas = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let fields = build_fields(
            &fields,
            &SourceModel::default(),
            &mut schemas,
            &mut diagnostics,
            "test.root",
            &TestPolicy,
        )
        .expect("fields build");
        assert_eq!(fields.len(), 5);
        assert!(fields[0].required);
        assert_eq!(fields[0].schema.default, Some(serde_json::json!(8080)));
        assert_eq!(fields[0].schema.description, "Listen port");
        assert!(fields[1].schema.sensitive);
        assert!(!fields[2].required);
        assert!(fields[2].flattened);
        assert!(matches!(fields[3].schema.kind, SchemaKind::Array { .. }));
        assert!(matches!(fields[4].schema.kind, SchemaKind::Map { .. }));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn builds_recursive_structs_and_tagged_enums() {
        let child = SourceStruct {
            name: "Child".to_owned(),
            doc: "A recursive child".to_owned(),
            fields: vec![field("next", "Option<Child>")],
            try_from: None,
        };
        let kind = SourceEnum {
            name: "Kind".to_owned(),
            doc: "Kinds".to_owned(),
            variants: vec![
                SourceEnumVariant {
                    name: "Text".to_owned(),
                    shape: SourceEnumVariantShape::Unit,
                },
                SourceEnumVariant {
                    name: "Number".to_owned(),
                    shape: SourceEnumVariantShape::Unnamed(vec![syn::parse_str("u64").expect("type")]),
                },
            ],
            tag: Some("kind".to_owned()),
            content: Some("value".to_owned()),
            untagged: false,
        };
        let root = SourceStruct {
            name: "Root".to_owned(),
            doc: String::new(),
            fields: vec![field("child", "Child"), field("kind", "Kind")],
            try_from: None,
        };
        let model = SourceModel {
            structs: [(child.name.clone(), child), (root.name.clone(), root)]
                .into_iter()
                .collect(),
            enums: [(kind.name.clone(), kind)].into_iter().collect(),
        };
        let mut schemas = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let fields = build_fields(
            &[field("root", "Root")],
            &model,
            &mut schemas,
            &mut diagnostics,
            "test",
            &TestPolicy,
        )
        .expect("recursive build");
        assert!(diagnostics.is_empty());
        assert!(matches!(fields[0].schema.kind, SchemaKind::Reference { .. }));
        let child_schema = schemas.get(&SchemaId::from("test.type.Child")).expect("child schema");
        let SchemaKind::Object { fields, .. } = &child_schema.node.kind else {
            panic!("child object")
        };
        assert!(matches!(fields[0].schema.kind, SchemaKind::Reference { .. }));
        let kind_schema = schemas.get(&SchemaId::from("test.type.Kind")).expect("kind schema");
        assert!(
            matches!(kind_schema.node.kind, SchemaKind::OneOf { discriminator: Some(ref tag), .. } if tag == "kind")
        );
    }

    #[test]
    fn reports_unknown_defaults_and_unresolved_types() {
        let mut defaulted = field("port", "u16");
        defaulted.default_path = Some("runtime::port".to_owned());
        let mut diagnostics = Vec::new();
        let mut schemas = BTreeMap::new();
        let fields = build_fields(
            &[defaulted],
            &SourceModel::default(),
            &mut schemas,
            &mut diagnostics,
            "test",
            &TestPolicy,
        )
        .expect("default field");
        assert_eq!(fields.len(), 1);
        assert_eq!(diagnostics[0].code, "unevaluable_default");
        let error = build_fields(
            &[field("bad", "Missing")],
            &SourceModel::default(),
            &mut schemas,
            &mut diagnostics,
            "test",
            &TestPolicy,
        )
        .expect_err("unknown type");
        assert!(error.message.contains("unresolved configuration type"));
    }
}

/// An error while committing a validated catalog artifact.
#[derive(Debug)]
pub enum ArtifactError {
    /// The producer fragment violated the catalog contract.
    Generation(GenerationError),
    /// The target artifact could not be written.
    Io {
        /// Target path that could not be written.
        path: PathBuf,
        /// Underlying filesystem error.
        source: io::Error,
    },
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Generation(error) => write!(f, "catalog generation failed: {error}"),
            Self::Io { path, source } => write!(f, "cannot write catalog artifact {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ArtifactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Generation(error) => Some(error),
            Self::Io { source, .. } => Some(source),
        }
    }
}

impl From<GenerationError> for ArtifactError {
    fn from(error: GenerationError) -> Self {
        Self::Generation(error)
    }
}

/// Validate a producer fragment.
pub fn validate(fragment: &CatalogFragment) -> Result<(), GenerationError> {
    praxis_config_catalog::generator::validate(fragment)
}

/// Finalize a fragment as deterministic, newline-terminated JSON.
pub fn render(fragment: CatalogFragment) -> Result<Vec<u8>, GenerationError> {
    praxis_config_catalog::generator::render(fragment)
}

/// Merge a Core fragment and extension fragments.
pub fn merge(core: CatalogFragment, extensions: Vec<CatalogFragment>) -> Result<MergedCatalog, MergeError> {
    praxis_config_catalog::generator::merge(core, extensions)
}

/// Render and write an artifact after successful validation.
pub fn write(fragment: CatalogFragment, path: impl AsRef<Path>) -> Result<(), ArtifactError> {
    let path = path.as_ref();
    let bytes = render(fragment)?;
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|source| ArtifactError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| ArtifactError::Io {
        path: path.to_path_buf(),
        source,
    })
}
