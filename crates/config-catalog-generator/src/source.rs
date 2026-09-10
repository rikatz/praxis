//! Producer-neutral Rust source parsing for configuration catalog generators.
//!
//! This module is intentionally concerned only with syntax and serde metadata.
//! It does not decide which declarations are configuration types, how filters
//! are discovered, or what schema a type means. Those decisions belong to the
//! small producer adapters that consume [`RustSourceModel`].

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

/// Parsed declarations collected from one or more Rust source files.
#[derive(Clone, Debug, Default)]
pub struct RustSourceModel {
    /// Module-level documentation from parsed files, in file order.
    pub module_docs: Vec<String>,
    /// Struct declarations keyed by their Rust identifier.
    pub structs: BTreeMap<String, RustStruct>,
    /// Enum declarations keyed by their Rust identifier.
    pub enums: BTreeMap<String, RustEnum>,
    /// Type aliases keyed by their local identifier.
    pub type_aliases: BTreeMap<String, String>,
    /// Single-field tuple structs keyed by their local identifier.
    pub newtypes: BTreeMap<String, syn::Type>,
    /// `serde(try_from = "...")` aliases keyed by their public identifier.
    pub try_from_aliases: BTreeMap<String, String>,
    /// Aliases that resolved to more than one source path.
    pub ambiguous_aliases: BTreeSet<String>,
    /// Files successfully parsed into this model.
    pub files: Vec<PathBuf>,
}

/// A parsed Rust struct and its serde-facing metadata.
#[derive(Clone, Debug)]
pub struct RustStruct {
    /// Rust identifier.
    pub name: String,
    /// Concatenated `///` documentation.
    pub docs: String,
    /// Fields in source order. Tuple fields are represented only when the
    /// struct has one field and therefore behaves as a serde newtype.
    pub fields: Vec<RustField>,
    /// Whether the item derives `Deserialize`.
    pub derives_deserialize: bool,
    /// Whether the item has `#[serde(deny_unknown_fields)]`.
    pub deny_unknown_fields: bool,
    /// Whether the item is public.
    pub public: bool,
    /// The source-level `serde(try_from = "...")` target, if present.
    pub try_from: Option<String>,
}

/// A parsed Rust field with serde and documentation metadata.
#[derive(Clone, Debug)]
pub struct RustField {
    /// Serialized field name after `serde(rename = ...)`.
    pub name: String,
    /// Rust type syntax.
    pub ty: syn::Type,
    /// Concatenated field documentation.
    pub docs: String,
    /// Accepted alternate serialized names.
    pub aliases: Vec<String>,
    /// Whether serde supplies a default.
    pub has_default: bool,
    /// Explicit default function path, if any.
    pub default_path: Option<String>,
    /// Custom serde deserializer path, if any.
    pub deserialize_with: Option<String>,
    /// Whether serde flattens this field into its containing object.
    pub flatten: bool,
    /// Whether serde skips this field while deserializing.
    pub skip: bool,
}

/// A parsed Rust enum.
#[derive(Clone, Debug)]
pub struct RustEnum {
    /// Rust identifier.
    pub name: String,
    /// Concatenated enum documentation.
    pub docs: String,
    /// Variants in source order.
    pub variants: Vec<RustEnumVariant>,
    /// Whether this enum derives `Deserialize`.
    pub derives_deserialize: bool,
    /// Whether serde uses untagged matching.
    pub untagged: bool,
    /// Internally tagged discriminator, if present.
    pub tag: Option<String>,
    /// Adjacently tagged content key, if present.
    pub content: Option<String>,
    /// `serde(rename_all = ...)`, preserved for adapters that need it.
    pub rename_all: Option<String>,
}

/// A parsed Rust enum variant.
#[derive(Clone, Debug)]
pub struct RustEnumVariant {
    /// Serialized variant name after `serde(rename = ...)` and `rename_all`.
    pub name: String,
    /// Variant documentation.
    pub docs: String,
    /// Variant payload shape.
    pub shape: RustEnumVariantShape,
}

/// The syntax shape of an enum variant.
#[derive(Clone, Debug)]
pub enum RustEnumVariantShape {
    /// A unit variant.
    Unit,
    /// Tuple fields in source order.
    Unnamed(Vec<syn::Type>),
    /// Named fields in source order.
    Named(Vec<RustField>),
}

/// A parse failure associated with a source file.
#[derive(Debug)]
pub enum SourceParseError {
    /// The file could not be read.
    Io {
        /// File path.
        path: PathBuf,
        /// Underlying filesystem failure.
        source: io::Error,
    },
    /// The file was not valid Rust syntax.
    Syntax {
        /// File path.
        path: PathBuf,
        /// Underlying parser failure.
        source: syn::Error,
    },
}

impl std::fmt::Display for SourceParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read Rust source {}: {source}", path.display()),
            Self::Syntax { path, source } => write!(f, "cannot parse Rust source {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for SourceParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Syntax { source, .. } => Some(source),
        }
    }
}

/// Recursively collect non-test Rust files below `root` in deterministic order.
pub fn collect_rust_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_files_inner(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_rust_files_inner(root: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_files_inner(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().is_none_or(|name| name != "tests.rs")
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Parse a deterministic set of Rust files into a normalized source model.
pub fn parse_rust_files<I>(paths: I) -> Result<RustSourceModel, SourceParseError>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut model = RustSourceModel::default();
    for path in paths {
        let source = fs::read_to_string(&path).map_err(|source| SourceParseError::Io {
            path: path.clone(),
            source,
        })?;
        let file = syn::parse_file(&source).map_err(|source| SourceParseError::Syntax {
            path: path.clone(),
            source,
        })?;
        parse_rust_file(&file, &mut model);
        model.files.push(path);
    }
    Ok(model)
}

/// Parse one Rust source string into an existing model.
pub fn parse_rust_source(source: &str, model: &mut RustSourceModel) -> syn::Result<()> {
    let file = syn::parse_file(source)?;
    parse_rust_file(&file, model);
    Ok(())
}

/// Parse a syn file into an existing model.
pub fn parse_rust_file(file: &syn::File, model: &mut RustSourceModel) {
    let docs = doc_comment(&file.attrs);
    if !docs.is_empty() {
        model.module_docs.push(docs);
    }

    let manual_deserializers = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(imp) if is_deserialize_impl(imp) => match &*imp.self_ty {
                syn::Type::Path(path) => path.path.segments.last().map(|segment| segment.ident.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    for item in &file.items {
        match item {
            syn::Item::Struct(item) => parse_struct(item, model),
            syn::Item::Enum(item) => parse_enum(item, model, manual_deserializers.contains(&item.ident.to_string())),
            syn::Item::Type(item) => {
                if let Some(target) = type_path_name(&item.ty) {
                    register_alias(model, item.ident.to_string(), target);
                }
            },
            syn::Item::Use(item) => collect_use_aliases(&item.tree, String::new(), model),
            _ => {},
        }
    }
}

fn parse_struct(item: &syn::ItemStruct, model: &mut RustSourceModel) {
    let try_from = serde_lit_value(&item.attrs, "try_from");
    let rename_all = serde_lit_value(&item.attrs, "rename_all");
    let fields = match &item.fields {
        syn::Fields::Named(fields) => fields
            .named
            .iter()
            .map(|field| parse_field(field, rename_all.as_deref()))
            .collect(),
        syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            if let Some(field) = fields.unnamed.first() {
                model.newtypes.insert(item.ident.to_string(), field.ty.clone());
            }
            Vec::new()
        },
        _ => Vec::new(),
    };
    if let Some(try_from) = &try_from {
        model.try_from_aliases.insert(item.ident.to_string(), try_from.clone());
    }
    model.structs.insert(
        item.ident.to_string(),
        RustStruct {
            name: item.ident.to_string(),
            docs: doc_comment(&item.attrs),
            fields,
            derives_deserialize: derives_deserialize(&item.attrs),
            deny_unknown_fields: has_serde_flag(&item.attrs, "deny_unknown_fields"),
            public: matches!(item.vis, syn::Visibility::Public(_)),
            try_from,
        },
    );
}

fn parse_enum(item: &syn::ItemEnum, model: &mut RustSourceModel, manual_deserialize: bool) {
    let rename_all = serde_lit_value(&item.attrs, "rename_all");
    let variants = item
        .variants
        .iter()
        .map(|variant| RustEnumVariant {
            name: variant
                .attrs
                .iter()
                .find_map(|attr| serde_lit_value_from_attr(attr, "rename"))
                .unwrap_or_else(|| apply_rename(&variant.ident.to_string(), rename_all.as_deref())),
            docs: doc_comment(&variant.attrs),
            shape: match &variant.fields {
                syn::Fields::Unit => RustEnumVariantShape::Unit,
                syn::Fields::Unnamed(fields) => {
                    RustEnumVariantShape::Unnamed(fields.unnamed.iter().map(|field| field.ty.clone()).collect())
                },
                syn::Fields::Named(fields) => {
                    RustEnumVariantShape::Named(fields.named.iter().map(|field| parse_field(field, None)).collect())
                },
            },
        })
        .collect();
    model.enums.insert(
        item.ident.to_string(),
        RustEnum {
            name: item.ident.to_string(),
            docs: doc_comment(&item.attrs),
            variants,
            derives_deserialize: derives_deserialize(&item.attrs) || manual_deserialize,
            untagged: has_serde_flag(&item.attrs, "untagged"),
            tag: serde_lit_value(&item.attrs, "tag"),
            content: serde_lit_value(&item.attrs, "content"),
            rename_all,
        },
    );
}

fn parse_field(field: &syn::Field, rename_all: Option<&str>) -> RustField {
    RustField {
        name: field
            .attrs
            .iter()
            .find_map(|attr| serde_lit_value_from_attr(attr, "rename"))
            .or_else(|| {
                field
                    .ident
                    .as_ref()
                    .map(|ident| apply_rename(&ident.to_string(), rename_all))
            })
            .unwrap_or_default(),
        ty: field.ty.clone(),
        docs: doc_comment(&field.attrs),
        aliases: field
            .attrs
            .iter()
            .flat_map(|attr| serde_lit_values_from_attr(attr, "alias"))
            .collect(),
        has_default: has_serde_flag(&field.attrs, "default"),
        default_path: serde_lit_value(&field.attrs, "default"),
        deserialize_with: serde_lit_value(&field.attrs, "deserialize_with"),
        flatten: has_serde_flag(&field.attrs, "flatten"),
        skip: has_serde_flag(&field.attrs, "skip") || has_serde_flag(&field.attrs, "skip_deserializing"),
    }
}

fn is_deserialize_impl(item: &syn::ItemImpl) -> bool {
    item.trait_.as_ref().is_some_and(|(path, _)| {
        path.segments
            .last()
            .is_some_and(|segment| segment.ident == "Deserialize")
    })
}

fn derives_deserialize(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("derive")
            && attr
                .parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
                .map(|paths| {
                    paths.iter().any(|path| {
                        path.segments
                            .last()
                            .is_some_and(|segment| segment.ident == "Deserialize")
                    })
                })
                .unwrap_or(false)
    })
}

fn doc_comment(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(value) if attr.path().is_ident("doc") => match &value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(value),
                    ..
                }) => Some(value.value()),
                _ => None,
            },
            _ => None,
        })
        .map(|line| line.strip_prefix(' ').unwrap_or(&line).to_owned())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn type_path_name(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = ty else { return None };
    path.path.segments.last().map(|segment| segment.ident.to_string())
}

fn register_alias(model: &mut RustSourceModel, alias: String, target: String) {
    if model
        .type_aliases
        .get(&alias)
        .is_some_and(|existing| existing != &target)
    {
        model.ambiguous_aliases.insert(alias.clone());
    }
    model
        .type_aliases
        .insert(alias, target.rsplit("::").next().unwrap_or(&target).to_owned());
}

fn collect_use_aliases(tree: &syn::UseTree, prefix: String, model: &mut RustSourceModel) {
    match tree {
        syn::UseTree::Path(path) => {
            let prefix = if prefix.is_empty() {
                path.ident.to_string()
            } else {
                format!("{prefix}::{}", path.ident)
            };
            collect_use_aliases(&path.tree, prefix, model);
        },
        syn::UseTree::Name(name) => {
            let target = if prefix.is_empty() {
                name.ident.to_string()
            } else {
                format!("{prefix}::{}", name.ident)
            };
            register_alias(model, name.ident.to_string(), target);
        },
        syn::UseTree::Rename(rename) => {
            let target = if prefix.is_empty() {
                rename.ident.to_string()
            } else {
                format!("{prefix}::{}", rename.ident)
            };
            register_alias(model, rename.rename.to_string(), target);
        },
        syn::UseTree::Group(group) => group
            .items
            .iter()
            .for_each(|item| collect_use_aliases(item, prefix.clone(), model)),
        syn::UseTree::Glob(_) => {},
    }
}

fn has_serde_flag(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("serde") {
            return false;
        }
        let mut found = false;
        drop(attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(name) {
                found = true;
            } else if meta.input.peek(syn::Token![=]) {
                drop(meta.value()?.parse::<syn::Expr>());
            }
            Ok(())
        }));
        found
    })
}

fn serde_lit_value(attrs: &[syn::Attribute], name: &str) -> Option<String> {
    attrs.iter().find_map(|attr| serde_lit_value_from_attr(attr, name))
}

fn serde_lit_value_from_attr(attr: &syn::Attribute, name: &str) -> Option<String> {
    if !attr.path().is_ident("serde") {
        return None;
    }
    let mut value = None;
    drop(attr.parse_nested_meta(|meta| {
        if meta.path.is_ident(name) {
            if let Ok(input) = meta.value() {
                if let Ok(literal) = input.parse::<syn::LitStr>() {
                    value = Some(literal.value());
                }
            }
        } else if meta.input.peek(syn::Token![=]) {
            drop(meta.value()?.parse::<syn::Expr>());
        }
        Ok(())
    }));
    value
}

fn serde_lit_values_from_attr(attr: &syn::Attribute, name: &str) -> Vec<String> {
    if !attr.path().is_ident("serde") {
        return Vec::new();
    }
    let mut values = Vec::new();
    drop(attr.parse_nested_meta(|meta| {
        if meta.path.is_ident(name) {
            if let Ok(input) = meta.value() {
                if let Ok(literal) = input.parse::<syn::LitStr>() {
                    values.push(literal.value());
                }
            }
        } else if meta.input.peek(syn::Token![=]) {
            drop(meta.value()?.parse::<syn::Expr>());
        }
        Ok(())
    }));
    values
}

fn apply_rename(name: &str, rule: Option<&str>) -> String {
    match rule {
        Some("snake_case") => to_snake_case(name),
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        Some("camelCase") => to_camel_case(name),
        Some("kebab-case") => to_snake_case(name).replace('_', "-"),
        Some("SCREAMING_SNAKE_CASE") => to_snake_case(name).to_uppercase(),
        Some("SCREAMING-KEBAB-CASE") => to_snake_case(name).to_uppercase().replace('_', "-"),
        _ => name.to_owned(),
    }
}

fn to_snake_case(name: &str) -> String {
    let mut out = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() && index > 0 {
            out.push('_');
        }
        out.extend(character.to_lowercase());
    }
    out
}

fn to_camel_case(name: &str) -> String {
    let mut out = String::new();
    let mut uppercase = false;
    for character in name.chars() {
        if character == '_' || character == '-' {
            uppercase = true;
            continue;
        }
        if uppercase {
            out.extend(character.to_uppercase());
            uppercase = false
        } else {
            out.push(character);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_serde_fields_and_enum_shapes() {
        let mut model = RustSourceModel::default();
        parse_rust_source(
            r#"
                use crate::shared::Port as ListenPort;
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields, rename_all = "kebab-case")]
                pub struct Config {
                    /// Port used by the listener.
                    #[serde(rename = "listen-port", alias = "port", default = "defaults::port")]
                    pub port: ListenPort,
                    pub idle_timeout: u64,
                    #[serde(flatten, skip_deserializing)]
                    extra: Extra,
                }
                #[derive(Deserialize)]
                #[serde(tag = "kind", content = "value")]
                enum Mode {
                    Fast,
                    Slow(u32),
                    #[serde(rename = "custom-mode")]
                    Custom { value: String },
                }
                struct Extra { value: String }
                struct Port(u16);
            "#,
            &mut model,
        )
        .expect("source parses");

        let config = model.structs.get("Config").expect("config");
        assert!(config.derives_deserialize);
        assert!(config.deny_unknown_fields);
        assert_eq!(config.fields[0].name, "listen-port");
        assert_eq!(config.fields[0].aliases, ["port"]);
        assert_eq!(config.fields[0].default_path.as_deref(), Some("defaults::port"));
        assert_eq!(config.fields[1].name, "idle-timeout");
        assert!(config.fields[2].flatten);
        assert!(config.fields[2].skip);
        assert_eq!(model.type_aliases.get("ListenPort"), Some(&"Port".to_owned()));
        assert!(model.newtypes.contains_key("Port"));

        let mode = model.enums.get("Mode").expect("mode");
        assert_eq!(mode.tag.as_deref(), Some("kind"));
        assert_eq!(mode.content.as_deref(), Some("value"));
        assert_eq!(mode.variants[2].name, "custom-mode");
        assert!(matches!(mode.variants[1].shape, RustEnumVariantShape::Unnamed(_)));
        assert!(matches!(mode.variants[2].shape, RustEnumVariantShape::Named(_)));
    }

    #[test]
    fn captures_manual_deserialize_and_try_from() {
        let mut model = RustSourceModel::default();
        parse_rust_source(
            r#"
                #[serde(try_from = "RawConfig")]
                pub struct Config { value: String }
                struct RawConfig { value: String }
                impl<'de> Deserialize<'de> for Mode {
                    fn deserialize<D>(_: D) -> Result<Self, D::Error> where D: Deserializer<'de> { todo!() }
                }
                enum Mode { Value }
            "#,
            &mut model,
        )
        .expect("source parses");
        assert_eq!(model.try_from_aliases.get("Config"), Some(&"RawConfig".to_owned()));
        assert!(model.enums.get("Mode").expect("mode").derives_deserialize);
    }
}
