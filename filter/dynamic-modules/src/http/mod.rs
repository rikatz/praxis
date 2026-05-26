// SPDX-License-Identifier: MIT

//! HTTP filter extension point for Envoy dynamic modules.

pub mod bridge;
#[allow(unreachable_pub, reason = "FFI callbacks must be pub for #[no_mangle] linking")]
pub(crate) mod callbacks;
pub(crate) mod context;
