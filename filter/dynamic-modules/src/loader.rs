// SPDX-License-Identifier: MIT

//! Dynamic module loader using `libloading` for `dlopen`.
//!
//! Loads an Envoy-compatible `.so` file, resolves the ABI entry
//! point and HTTP filter symbols, and validates the ABI version.

use std::{ffi::CStr, path::Path, sync::Arc};

use libloading::{Library, Symbol};
use praxis_filter::FilterError;

use crate::abi::{
    ConfigEnvoyPtr, ConfigModulePtr, EnvoyBuffer, FilterEnvoyPtr, FilterModulePtr, RequestHeadersStatus,
    ResponseHeadersStatus,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Expected ABI version prefix. We accept any `v0.*` version for the
/// initial implementation; a production build should match exactly.
const ABI_VERSION_PREFIX: &str = "v0.";

// ---------------------------------------------------------------------------
// Function Pointer Types
// ---------------------------------------------------------------------------

/// `envoy_dynamic_module_on_program_init() -> *const c_char`
type ProgramInitFn = unsafe extern "C" fn() -> *const std::os::raw::c_char;

/// `envoy_dynamic_module_on_http_filter_config_new(...) -> config_module_ptr`
type ConfigNewFn = unsafe extern "C" fn(ConfigEnvoyPtr, EnvoyBuffer, EnvoyBuffer) -> ConfigModulePtr;

/// `envoy_dynamic_module_on_http_filter_config_destroy(config_module_ptr)`
type ConfigDestroyFn = unsafe extern "C" fn(ConfigModulePtr);

/// `envoy_dynamic_module_on_http_filter_new(config_module_ptr, filter_envoy_ptr) -> filter_module_ptr`
type FilterNewFn = unsafe extern "C" fn(ConfigModulePtr, FilterEnvoyPtr) -> FilterModulePtr;

/// `envoy_dynamic_module_on_http_filter_destroy(filter_module_ptr)`
type FilterDestroyFn = unsafe extern "C" fn(FilterModulePtr);

/// `envoy_dynamic_module_on_http_filter_request_headers(...) -> status`
type RequestHeadersFn = unsafe extern "C" fn(FilterEnvoyPtr, FilterModulePtr, bool) -> RequestHeadersStatus;

/// `envoy_dynamic_module_on_http_filter_response_headers(...) -> status`
type ResponseHeadersFn = unsafe extern "C" fn(FilterEnvoyPtr, FilterModulePtr, bool) -> ResponseHeadersStatus;

// ---------------------------------------------------------------------------
// HttpFilterSymbols
// ---------------------------------------------------------------------------

/// Resolved HTTP filter ABI symbols from a loaded module.
pub(crate) struct HttpFilterSymbols {
    /// Creates a per-listener config object.
    pub(crate) config_new: ConfigNewFn,
    /// Destroys the config object.
    pub(crate) config_destroy: ConfigDestroyFn,
    /// Creates a per-stream filter instance.
    pub(crate) filter_new: FilterNewFn,
    /// Destroys a per-stream filter instance.
    pub(crate) filter_destroy: FilterDestroyFn,
    /// Request headers hook (optional).
    pub(crate) request_headers: Option<RequestHeadersFn>,
    /// Response headers hook (optional).
    pub(crate) response_headers: Option<ResponseHeadersFn>,
}

// ---------------------------------------------------------------------------
// DynamicModule
// ---------------------------------------------------------------------------

/// A loaded Envoy dynamic module (`.so` file).
///
/// Owns the `libloading::Library` handle to keep the shared object
/// mapped for the process lifetime. Resolved symbols are stored as
/// plain function pointers (safe to share across threads once
/// resolved).
pub(crate) struct DynamicModule {
    /// The loaded library handle. Must outlive all symbol references.
    _library: Library,
    /// HTTP filter symbols (populated if the module exports them).
    pub(crate) http: Option<HttpFilterSymbols>,
}

// SAFETY: The function pointers stored here are `extern "C"` static
// symbols from a loaded `.so`. They do not carry thread-local state
// and are safe to call from any thread (the Envoy ABI requires
// modules to be thread-safe).
unsafe impl Send for DynamicModule {}
// SAFETY: See above.
unsafe impl Sync for DynamicModule {}

impl DynamicModule {
    /// Load a dynamic module from the given path.
    ///
    /// Calls `envoy_dynamic_module_on_program_init` and validates the
    /// returned ABI version. Resolves HTTP filter symbols if the
    /// module exports `envoy_dynamic_module_on_http_filter_config_new`.
    ///
    /// # Errors
    ///
    /// Returns [`FilterError`] if the library cannot be opened, the
    /// init symbol is missing, or the module returns a null ABI
    /// version.
    #[allow(clippy::too_many_lines, reason = "FFI init sequence is inherently sequential")]
    pub(crate) fn load(path: &Path) -> Result<Arc<Self>, FilterError> {
        // SAFETY: Loading a shared library is inherently unsafe. We
        // trust the operator to provide a valid `.so` built against
        // the Envoy dynamic module ABI.
        let library = unsafe { Library::new(path) }
            .map_err(|e| format!("failed to load dynamic module '{}': {e}", path.display()))?;

        // --- program_init ---
        let program_init = resolve_required::<ProgramInitFn>(&library, b"envoy_dynamic_module_on_program_init\0")?;

        // SAFETY: program_init is the module's global init function.
        let abi_version_ptr = unsafe { program_init() };
        if abi_version_ptr.is_null() {
            return Err(format!(
                "dynamic module '{}': program_init returned null (init failed)",
                path.display()
            )
            .into());
        }

        // SAFETY: The ABI guarantees the returned pointer is a valid
        // null-terminated C string that remains valid until
        // program_init returns (it already returned).
        let abi_version = unsafe { CStr::from_ptr(abi_version_ptr) }
            .to_str()
            .unwrap_or("<invalid UTF-8>");

        if abi_version.starts_with(ABI_VERSION_PREFIX) {
            tracing::info!(
                module = %path.display(),
                abi_version,
                "Dynamic module loaded"
            );
        } else {
            tracing::warn!(
                module = %path.display(),
                abi_version,
                expected_prefix = ABI_VERSION_PREFIX,
                "Dynamic module ABI version mismatch (proceeding anyway)"
            );
        }

        // --- HTTP filter symbols ---
        let http = resolve_http_symbols(&library, path)?;

        Ok(Arc::new(Self {
            _library: library,
            http,
        }))
    }
}

// ---------------------------------------------------------------------------
// Symbol Resolution Helpers
// ---------------------------------------------------------------------------

/// Resolve a required symbol or return an error.
fn resolve_required<T>(library: &Library, name: &[u8]) -> Result<T, FilterError>
where
    T: Copy,
{
    // SAFETY: We are resolving a symbol from the loaded library.
    // The caller ensures the type `T` matches the actual symbol.
    let sym: Symbol<'_, T> = unsafe { library.get(name) }.map_err(|e| {
        let name_str = String::from_utf8_lossy(name.strip_suffix(b"\0").unwrap_or(name));
        format!("missing required symbol '{name_str}': {e}")
    })?;
    Ok(*sym)
}

/// Resolve an optional symbol; returns `None` if not found.
fn resolve_optional<T>(library: &Library, name: &[u8]) -> Option<T>
where
    T: Copy,
{
    // SAFETY: Same as resolve_required.
    unsafe { library.get::<T>(name) }.ok().map(|s| *s)
}

/// Attempt to resolve HTTP filter symbols from the library.
///
/// Returns `None` if `config_new` is not exported (the module does
/// not implement HTTP filters). Returns an error if `config_new`
/// exists but other required symbols are missing.
#[allow(clippy::too_many_lines, reason = "FFI symbol resolution is inherently sequential")]
fn resolve_http_symbols(library: &Library, path: &Path) -> Result<Option<HttpFilterSymbols>, FilterError> {
    let config_new: Option<ConfigNewFn> =
        resolve_optional(library, b"envoy_dynamic_module_on_http_filter_config_new\0");

    let Some(config_new) = config_new else {
        return Ok(None);
    };

    let config_destroy: ConfigDestroyFn =
        resolve_required(library, b"envoy_dynamic_module_on_http_filter_config_destroy\0")
            .map_err(|e| format!("module '{}': {e}", path.display()))?;

    let filter_new: FilterNewFn = resolve_required(library, b"envoy_dynamic_module_on_http_filter_new\0")
        .map_err(|e| format!("module '{}': {e}", path.display()))?;

    let filter_destroy: FilterDestroyFn = resolve_required(library, b"envoy_dynamic_module_on_http_filter_destroy\0")
        .map_err(|e| format!("module '{}': {e}", path.display()))?;

    let request_headers: Option<RequestHeadersFn> =
        resolve_optional(library, b"envoy_dynamic_module_on_http_filter_request_headers\0");

    let response_headers: Option<ResponseHeadersFn> =
        resolve_optional(library, b"envoy_dynamic_module_on_http_filter_response_headers\0");

    Ok(Some(HttpFilterSymbols {
        config_new,
        config_destroy,
        filter_new,
        filter_destroy,
        request_headers,
        response_headers,
    }))
}
