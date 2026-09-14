// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Manual trait impls for standard library and common types.
//!
//! These types do not derive ConfigSchemaFor and instead provide hand-written
//! impls that reflect their runtime semantics (e.g., constrained numeric
//! ranges, custom deserializers).

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

use crate::{ConfigSchema, ConfigSchemaFor, SchemaId, SchemaKind, SchemaNode};

// Primitive types

macro_rules! scalar_schema {
    ($($ty:ty => ($id:literal, $kind:expr)),+ $(,)?) => {$ (
        impl ConfigSchemaFor for $ty {
            fn schema_id() -> SchemaId { SchemaId::from($id) }
            fn register(
                _schemas: &mut BTreeMap<SchemaId, ConfigSchema>,
                _visiting: &mut BTreeSet<SchemaId>,
            ) -> SchemaNode {
                SchemaNode::simple($kind)
            }
        }
    )+ };
}

scalar_schema! {
    bool => ("core.bool", SchemaKind::Boolean),
    u8 => ("core.u8", SchemaKind::Integer),
    u16 => ("core.u16", SchemaKind::Integer),
    u32 => ("core.u32", SchemaKind::Integer),
    u64 => ("core.u64", SchemaKind::Integer),
    u128 => ("core.u128", SchemaKind::Integer),
    usize => ("core.usize", SchemaKind::Integer),
    i8 => ("core.i8", SchemaKind::Integer),
    i16 => ("core.i16", SchemaKind::Integer),
    i32 => ("core.i32", SchemaKind::Integer),
    i64 => ("core.i64", SchemaKind::Integer),
    i128 => ("core.i128", SchemaKind::Integer),
    isize => ("core.isize", SchemaKind::Integer),
    f32 => ("core.f32", SchemaKind::Number),
    f64 => ("core.f64", SchemaKind::Number),
    String => ("core.string", SchemaKind::String),
    str => ("core.string", SchemaKind::String),
}

impl ConfigSchemaFor for PathBuf {
    fn schema_id() -> SchemaId {
        SchemaId::from("core.pathbuf")
    }

    fn register(_schemas: &mut BTreeMap<SchemaId, ConfigSchema>, _visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        SchemaNode::simple(SchemaKind::String)
    }
}

impl ConfigSchemaFor for Duration {
    fn schema_id() -> SchemaId {
        SchemaId::from("core.duration")
    }

    fn register(_schemas: &mut BTreeMap<SchemaId, ConfigSchema>, _visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        SchemaNode::simple(SchemaKind::String)
    }
}

impl ConfigSchemaFor for () {
    fn schema_id() -> SchemaId {
        SchemaId::from("core.null")
    }

    fn register(_schemas: &mut BTreeMap<SchemaId, ConfigSchema>, _visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        SchemaNode::simple(SchemaKind::Null)
    }
}

// Generic impls

impl<T: ConfigSchemaFor> ConfigSchemaFor for Option<T> {
    fn schema_id() -> SchemaId {
        T::schema_id()
    }

    fn register(schemas: &mut BTreeMap<SchemaId, ConfigSchema>, visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        T::register(schemas, visiting)
    }
}

impl<T: ConfigSchemaFor> ConfigSchemaFor for Box<T> {
    fn schema_id() -> SchemaId {
        T::schema_id()
    }

    fn register(schemas: &mut BTreeMap<SchemaId, ConfigSchema>, visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        T::register(schemas, visiting)
    }
}

impl<T: ConfigSchemaFor + ?Sized> ConfigSchemaFor for std::sync::Arc<T> {
    fn schema_id() -> SchemaId {
        T::schema_id()
    }

    fn register(schemas: &mut BTreeMap<SchemaId, ConfigSchema>, visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        T::register(schemas, visiting)
    }
}

impl<T: ConfigSchemaFor> ConfigSchemaFor for Vec<T> {
    fn schema_id() -> SchemaId {
        SchemaId::from(format!("core.vec.{}", T::schema_id().0))
    }

    fn register(schemas: &mut BTreeMap<SchemaId, ConfigSchema>, visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        let item_node = T::register(schemas, visiting);
        SchemaNode::array(item_node)
    }
}

impl<T: ConfigSchemaFor> ConfigSchemaFor for BTreeMap<String, T> {
    fn schema_id() -> SchemaId {
        SchemaId::from(format!("core.btreemap.{}", T::schema_id().0))
    }

    fn register(schemas: &mut BTreeMap<SchemaId, ConfigSchema>, visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        let value_node = T::register(schemas, visiting);
        SchemaNode::map(value_node)
    }
}

impl<T: ConfigSchemaFor> ConfigSchemaFor for std::collections::HashMap<String, T> {
    fn schema_id() -> SchemaId {
        SchemaId::from(format!("core.hashmap.{}", T::schema_id().0))
    }

    fn register(schemas: &mut BTreeMap<SchemaId, ConfigSchema>, visiting: &mut BTreeSet<SchemaId>) -> SchemaNode {
        let value_node = T::register(schemas, visiting);
        SchemaNode::map(value_node)
    }
}
