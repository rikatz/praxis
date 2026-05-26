// SPDX-License-Identifier: MIT

//! Envoy dynamic module bridge for Praxis.
//!
//! Loads Envoy-compatible `.so` modules via `dlopen` and translates
//! between the Envoy dynamic module C ABI and Praxis's filter traits.
//!
//! # Feature Flags
//!
//! - `regenerate-abi`: Reruns `bindgen` on the vendored `abi/abi.h` to regenerate `src/abi_generated.rs`. Requires
//!   `libclang`. Normal builds use the pre-generated file.

#![allow(unsafe_code, reason = "FFI bridge requires unsafe for dlopen and C ABI calls")]

pub(crate) mod abi;
pub(crate) mod abi_sys;
mod config;
pub mod http;
mod loader;

pub use config::DynamicModuleConfig;
pub use http::bridge::EnvoyDynamicModuleFilter;
