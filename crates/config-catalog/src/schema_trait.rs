// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Trait for types that can produce configuration schemas.
//!
//! # Trait naming: ConfigSchemaFor vs HasConfigSchema
//!
//! Named ConfigSchemaFor to avoid collision with the existing ConfigSchema
//! struct in this crate (which represents a complete schema definition with id,
//! title, node, etc.). The derive macro generates impls of this trait; consumers
//! write one `use praxis_config_catalog::ConfigSchemaFor;` and then use
//! `#[derive(ConfigSchemaFor)]` on their types.
//!
//! This is the same naming pattern used by Deserialize (trait) vs the struct
//! shapes it operates on.

use std::collections::{BTreeMap, BTreeSet};

use super::{ConfigSchema, SchemaId, SchemaNode};

/// A type that can produce a configuration schema node.
///
/// Implementors must provide a stable `SchemaId` and build their schema by
/// recursively registering any referenced types into the provided `schemas` map.
///
/// # Idempotency and cycle breaking
///
/// `register()` must be idempotent: calling it twice on the same set must not
/// panic or duplicate entries. The `visiting` set breaks reference cycles:
/// if `Self::schema_id()` is already in `visiting` (currently being expanded on
/// the call stack) or already a key in `schemas`, return
/// `SchemaNode::reference(id.0)` immediately instead of recursing.
///
/// # Example
///
/// ```ignore
/// impl ConfigSchemaFor for MyType {
///     fn schema_id() -> SchemaId {
///         SchemaId::from("core.my.type")
///     }
///
///     fn register(
///         schemas: &mut BTreeMap<SchemaId, ConfigSchema>,
///         visiting: &mut BTreeSet<SchemaId>,
///     ) -> SchemaNode {
///         let id = Self::schema_id();
///         if visiting.contains(&id) || schemas.contains_key(&id) {
///             return SchemaNode::reference(id.0);
///         }
///
///         visiting.insert(id.clone());
///         // Build schema...
///         visiting.remove(&id);
///
///         SchemaNode::reference(id.0)
///     }
/// }
/// ```
pub trait ConfigSchemaFor {
    /// Stable, producer-prefixed schema id. Two types must never return the
    /// same id unless they are the same wire shape.
    fn schema_id() -> SchemaId;

    /// Build this type's `SchemaNode`, inserting itself (and anything it
    /// references) into `schemas`. Idempotent. `visiting` breaks reference
    /// cycles: a type already being expanded on the current call stack
    /// returns a bare `SchemaNode::reference(id)` instead of recursing.
    fn register(schemas: &mut BTreeMap<SchemaId, ConfigSchema>, visiting: &mut BTreeSet<SchemaId>) -> SchemaNode;
}
