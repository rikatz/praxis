// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! Shared live pipeline state used by `/ready` and admin stats.

use std::sync::Arc;

use praxis_core::health::HealthRegistry;

use super::listener_meta::ListenerMetaStore;
use crate::ListenerPipelines;

/// Pipeline and metadata handles used to resolve current readiness.
#[derive(Clone)]
pub(super) struct PipelineReadinessState {
    /// Live per-listener pipelines.
    pub pipelines: Arc<ListenerPipelines>,

    /// Hot-swappable listener metadata.
    pub meta: ListenerMetaStore,
}

/// Resolve the current health registry from live pipelines when available.
pub(super) fn resolve_health_registry(
    admin_registry: Option<&HealthRegistry>,
    pipelines: Option<&PipelineReadinessState>,
    listener_meta: &ListenerMetaStore,
) -> Option<HealthRegistry> {
    let Some(state) = pipelines else {
        return admin_registry.cloned();
    };
    let meta = listener_meta.load();
    let current_listeners: std::collections::HashSet<String> = meta.keys().cloned().collect();
    for name in state.pipelines.listener_names() {
        if !current_listeners.contains(name) {
            continue;
        }
        let Some(slot) = state.pipelines.get(name) else {
            continue;
        };
        if let Some(registry) = slot.load().health_registry() {
            return Some(Arc::clone(registry));
        }
    }
    // No live pipeline exposes a registry (e.g. health checks removed on reload).
    // Do not fall back to the startup admin snapshot — it would report stale probe state.
    None
}
