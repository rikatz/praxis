// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Procedural macro for deriving `ConfigSchemaFor`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::quote;
use syn::{Attribute, Data, DataEnum, DataStruct, DeriveInput, Fields, LitStr, Type, parse::Parse, parse_macro_input};

/// Derive a recursive configuration schema from a serde-shaped struct or enum.
#[proc_macro_derive(ConfigSchemaFor, attributes(config_schema, serde))]
pub fn derive_config_schema_for(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input).unwrap_or_else(syn::Error::into_compile_error).into()
}

fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
    let id = schema_id(&input.attrs)?;
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let description = docs(&input.attrs);
    let rename_all = serde_rename_all(&input.attrs);
    let body = match &input.data {
        Data::Struct(data) => struct_body(data, rename_all.as_deref())?,
        Data::Enum(data) => enum_body(data, rename_all.as_deref(), serde_tag(&input.attrs).as_deref())?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "ConfigSchemaFor cannot derive for unions",
            ));
        },
    };

    Ok(quote! {
        impl #impl_generics praxis_config_catalog::ConfigSchemaFor for #name #ty_generics #where_clause {
            fn schema_id() -> praxis_config_catalog::SchemaId {
                praxis_config_catalog::SchemaId::from(#id)
            }

            fn register(
                schemas: &mut std::collections::BTreeMap<praxis_config_catalog::SchemaId, praxis_config_catalog::ConfigSchema>,
                visiting: &mut std::collections::BTreeSet<praxis_config_catalog::SchemaId>,
            ) -> praxis_config_catalog::SchemaNode {
                let id = Self::schema_id();
                if visiting.contains(&id) || schemas.contains_key(&id) {
                    return praxis_config_catalog::SchemaNode::reference(id.0.clone());
                }
                visiting.insert(id.clone());
                let node = { #body };
                visiting.remove(&id);
                schemas.insert(id.clone(), praxis_config_catalog::ConfigSchema {
                    id: id.clone(),
                    title: stringify!(#name).to_owned(),
                    description: #description.to_owned(),
                    node,
                    shared: false,
                    producer: None,
                });
                praxis_config_catalog::SchemaNode::reference(id.0)
            }
        }
    })
}

fn struct_body(data: &DataStruct, rename_all: Option<&str>) -> syn::Result<Tokens> {
    let Fields::Named(fields) = &data.fields else {
        return match &data.fields {
            Fields::Unit => Ok(quote! { praxis_config_catalog::SchemaNode::object(Vec::new()) }),
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let ty = &fields.unnamed[0].ty;
                Ok(quote! { <#ty as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting) })
            },
            _ => Err(syn::Error::new_spanned(
                &data.fields,
                "ConfigSchemaFor requires named fields or a single-field newtype",
            )),
        };
    };
    let fields = fields.named.iter().filter_map(|field| {
        if has_flag(&field.attrs, "skip") || has_flag(&field.attrs, "skip_deserializing") {
            return None;
        }
        let ident = field.ident.as_ref().expect("named field");
        let ty = &field.ty;
        let name = serialized_name(&field.attrs, &ident.to_string(), rename_all);
        let aliases = aliases(&field.attrs);
        let required = !has_default(&field.attrs) && !is_option(ty) && !has_flag(&field.attrs, "flatten");
        let flattened = has_flag(&field.attrs, "flatten");
        let schema = if has_config_flag(&field.attrs, "dynamic") {
            quote! { praxis_config_catalog::SchemaNode::reference("core.filter.entry.config".to_owned()) }
        } else {
            quote! { <#ty as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting) }
        };
        Some(quote! {
            fields.push(praxis_config_catalog::ObjectField {
                serialized_name: #name.to_owned(),
                aliases: vec![#(#aliases.to_owned()),*],
                schema: #schema,
                required: #required,
                flattened: #flattened,
            });
        })
    });
    Ok(quote! {
        let mut fields = Vec::new();
        #(#fields)*
        praxis_config_catalog::SchemaNode::object(fields)
    })
}

fn enum_body(data: &DataEnum, rename_all: Option<&str>, tag: Option<&str>) -> syn::Result<Tokens> {
    let mut unit_values = Vec::new();
    let mut variants = Vec::new();
    for variant in &data.variants {
        let name = serialized_name(&variant.attrs, &variant.ident.to_string(), rename_all);
        match &variant.fields {
            Fields::Unit => {
                if let Some(tag) = tag {
                    variants.push(quote! {
                        variants.push(praxis_config_catalog::SchemaNode::object(vec![
                            praxis_config_catalog::ObjectField {
                                serialized_name: #tag.to_owned(),
                                aliases: Vec::new(),
                                schema: praxis_config_catalog::SchemaNode::literal(serde_json::Value::String(#name.to_owned())),
                                required: true,
                                flattened: false,
                            },
                        ]));
                    });
                } else {
                    unit_values.push(name);
                }
            },
            Fields::Named(fields) => {
                let field_tokens = fields.named.iter().filter_map(|field| {
                    let ident = field.ident.as_ref().expect("named field");
                    if has_flag(&field.attrs, "skip") || has_flag(&field.attrs, "skip_deserializing") {
                        return None;
                    }
                    let ty = &field.ty;
                    let name = serialized_name(&field.attrs, &ident.to_string(), None);
                    let aliases = aliases(&field.attrs);
                    let required = !has_default(&field.attrs) && !is_option(ty);
                    Some(quote! {
                        variant_fields.push(praxis_config_catalog::ObjectField {
                            serialized_name: #name.to_owned(),
                            aliases: vec![#(#aliases.to_owned()),*],
                            schema: <#ty as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting),
                            required: #required,
                            flattened: false,
                        });
                    })
                });
                let tag_field = tag.map(|tag| quote! {
                    variant_fields.push(praxis_config_catalog::ObjectField {
                        serialized_name: #tag.to_owned(),
                        aliases: Vec::new(),
                        schema: praxis_config_catalog::SchemaNode::literal(serde_json::Value::String(#name.to_owned())),
                        required: true,
                        flattened: false,
                    });
                });
                variants.push(quote! {
                    let mut variant_fields = Vec::new();
                    #tag_field
                    #(#field_tokens)*
                    variants.push(praxis_config_catalog::SchemaNode::object(variant_fields));
                });
            },
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let ty = &fields.unnamed[0].ty;
                variants.push(quote! {
                    variants.push(<#ty as praxis_config_catalog::ConfigSchemaFor>::register(schemas, visiting));
                });
            },
            Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    &variant.fields,
                    "tuple enum variants must contain one field",
                ));
            },
        }
    }
    if variants.is_empty() {
        Ok(quote! { praxis_config_catalog::SchemaNode::enum_strings(vec![#(#unit_values.to_owned()),*]) })
    } else {
        let discriminator = tag
            .map(|tag| quote! { Some(#tag.to_owned()) })
            .unwrap_or_else(|| quote! { None });
        Ok(quote! {
            let mut variants = Vec::new();
            #(#variants)*
            praxis_config_catalog::SchemaNode::one_of_with_discriminator(variants, #discriminator)
        })
    }
}

fn schema_id(attrs: &[Attribute]) -> syn::Result<String> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("config_schema")) {
        let value: LitStr = attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
            let key: syn::Ident = input.parse()?;
            if key != "id" {
                return Err(syn::Error::new(key.span(), "only `id` is supported"));
            }
            input.parse::<syn::Token![=]>()?;
            input.parse()
        })?;
        return Ok(value.value());
    }
    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "missing #[config_schema(id = \"...\")]",
    ))
}

fn serde_items(attr: &Attribute) -> syn::Result<syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>> {
    attr.parse_args_with(|input: syn::parse::ParseStream<'_>| input.parse_terminated(syn::Meta::parse, syn::Token![,]))
}

fn serialized_name(attrs: &[Attribute], fallback: &str, rename_all: Option<&str>) -> String {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
        .find_map(|attr| {
            serde_items(attr).ok()?.into_iter().find_map(|item| match item {
                syn::Meta::NameValue(value) if value.path.is_ident("rename") => match value.value {
                    syn::Expr::Lit(expr) => match expr.lit {
                        syn::Lit::Str(value) => Some(value.value()),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            })
        })
        .unwrap_or_else(|| rename_all.map_or_else(|| fallback.to_owned(), |rule| apply_rename(fallback, rule)))
}

fn serde_rename_all(attrs: &[Attribute]) -> Option<String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
        .find_map(|attr| {
            serde_items(attr).ok()?.into_iter().find_map(|item| match item {
                syn::Meta::NameValue(value) if value.path.is_ident("rename_all") => match value.value {
                    syn::Expr::Lit(expr) => match expr.lit {
                        syn::Lit::Str(value) => Some(value.value()),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            })
        })
}

fn serde_tag(attrs: &[Attribute]) -> Option<String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
        .find_map(|attr| {
            serde_items(attr).ok()?.into_iter().find_map(|item| match item {
                syn::Meta::NameValue(value) if value.path.is_ident("tag") => match value.value {
                    syn::Expr::Lit(expr) => match expr.lit {
                        syn::Lit::Str(value) => Some(value.value()),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            })
        })
}

fn apply_rename(name: &str, rule: &str) -> String {
    let mut words = Vec::new();
    let mut word = String::new();
    for character in name.chars() {
        if matches!(character, '_' | '-') {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            if character.is_uppercase() && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            word.push(character.to_ascii_lowercase());
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    let capitalized = |word: &str| {
        let mut chars = word.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_ascii_uppercase().to_string() + chars.as_str()
        })
    };
    match rule {
        "camelCase" => {
            words.first().cloned().unwrap_or_default()
                + &words.iter().skip(1).map(|word| capitalized(word)).collect::<String>()
        },
        "PascalCase" => words.iter().map(|word| capitalized(word)).collect(),
        "kebab-case" => words.join("-"),
        "SCREAMING_SNAKE_CASE" => words.join("_").to_ascii_uppercase(),
        _ => words.join("_"),
    }
}

fn aliases(attrs: &[Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
        .filter_map(|attr| {
            serde_items(attr).ok().map(|items| {
                items
                    .into_iter()
                    .filter_map(|item| match item {
                        syn::Meta::NameValue(value) if value.path.is_ident("alias") => match value.value {
                            syn::Expr::Lit(expr) => match expr.lit {
                                syn::Lit::Str(value) => Some(value.value()),
                                _ => None,
                            },
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect()
}

fn has_flag(attrs: &[Attribute], flag: &str) -> bool {
    attrs.iter().filter(|attr| attr.path().is_ident("serde")).any(|attr| {
        serde_items(attr)
            .map(|items| {
                items
                    .into_iter()
                    .any(|item| matches!(item, syn::Meta::Path(path) if path.is_ident(flag)))
            })
            .unwrap_or(false)
    })
}

fn has_config_flag(attrs: &[Attribute], flag: &str) -> bool {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("config_schema"))
        .any(|attr| {
            attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
                let items = input.parse_terminated(syn::Meta::parse, syn::Token![,])?;
                Ok(items
                    .into_iter()
                    .any(|item| matches!(item, syn::Meta::Path(path) if path.is_ident(flag))))
            })
            .unwrap_or(false)
        })
}

fn has_default(attrs: &[Attribute]) -> bool {
    attrs.iter().filter(|attr| attr.path().is_ident("serde")).any(|attr| {
        serde_items(attr).is_ok_and(|items| {
            items.into_iter().any(|item| match item {
                syn::Meta::Path(path) => path.is_ident("default"),
                syn::Meta::NameValue(value) => value.path.is_ident("default"),
                syn::Meta::List(_) => false,
            })
        })
    })
}

fn is_option(ty: &Type) -> bool {
    let Type::Path(path) = ty else { return false };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Option")
}

fn docs(attrs: &[Attribute]) -> String {
    attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            attr.parse_args::<LitStr>()
                .ok()
                .map(|value| value.value().trim().to_owned())
        })
        .collect::<Vec<_>>()
        .join("\n")
}
