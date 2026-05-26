// SPDX-License-Identifier: MIT

//! Bridge filter: implements Praxis [`HttpFilter`] by delegating to an
//! Envoy dynamic module.
//!
//! [`HttpFilter`]: praxis_filter::HttpFilter

use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use praxis_filter::{FilterAction, HttpFilter, HttpFilterContext, Rejection};

use super::context::EnvoyHttpFilterContext;
use crate::{
    abi::{REQUEST_HEADERS_CONTINUE, RESPONSE_HEADERS_CONTINUE, envoy_buffer_from_slice},
    config::DynamicModuleConfig,
    loader::DynamicModule,
};

// ---------------------------------------------------------------------------
// FilterDestroyGuard
// ---------------------------------------------------------------------------

/// RAII guard that calls `filter_destroy` on drop, ensuring cleanup
/// even if the caller panics after `filter_new`.
struct FilterDestroyGuard {
    /// The destroy function pointer.
    destroy_fn: unsafe extern "C" fn(crate::abi::FilterModulePtr),
    /// The module-side filter pointer to destroy.
    filter_ptr: crate::abi::FilterModulePtr,
}

impl Drop for FilterDestroyGuard {
    fn drop(&mut self) {
        // SAFETY: filter_ptr was returned by filter_new and is valid
        // until filter_destroy is called.
        unsafe { (self.destroy_fn)(self.filter_ptr) };
    }
}

// ---------------------------------------------------------------------------
// EnvoyDynamicModuleFilter
// ---------------------------------------------------------------------------

/// A Praxis HTTP filter backed by an Envoy dynamic module `.so`.
///
/// Created at config time (one per filter entry in the pipeline).
/// Shared across all requests on the listener.
pub struct EnvoyDynamicModuleFilter {
    /// The loaded dynamic module (shared via `Arc`).
    module: Arc<DynamicModule>,
    /// Module-side config pointer returned by `config_new`.
    config_ptr: crate::abi::ConfigModulePtr,
    /// Display name for logging.
    filter_name: String,
}

// SAFETY: config_ptr is an opaque pointer to module-managed memory.
// The Envoy ABI requires modules to be thread-safe: config objects
// are created on the main thread and accessed from worker threads.
unsafe impl Send for EnvoyDynamicModuleFilter {}
// SAFETY: See above.
unsafe impl Sync for EnvoyDynamicModuleFilter {}

impl EnvoyDynamicModuleFilter {
    /// Construct from YAML config.
    ///
    /// Loads the `.so`, calls `program_init` (via the loader),
    /// then `config_new` to create the module-side config.
    ///
    /// # Errors
    ///
    /// Returns [`FilterError`] if the module cannot be loaded, does
    /// not export HTTP filter symbols, or `config_new` fails.
    ///
    /// [`FilterError`]: praxis_filter::FilterError
    #[allow(clippy::too_many_lines, reason = "FFI config setup is inherently sequential")]
    pub fn from_config(config: &serde_yaml::Value) -> Result<Box<dyn HttpFilter>, praxis_filter::FilterError> {
        let cfg: DynamicModuleConfig = praxis_filter::parse_filter_config("envoy_dynamic_module", config)?;

        let path = Path::new(&cfg.module_path);
        let module = DynamicModule::load(path)?;

        let http_symbols = module
            .http
            .as_ref()
            .ok_or("dynamic module does not export HTTP filter symbols")?;

        let name = cfg.module_name.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_owned()
        });

        let module_config = cfg.module_config.unwrap_or_default();

        let name_buf = envoy_buffer_from_slice(name.as_bytes());
        let config_buf = envoy_buffer_from_slice(module_config.as_bytes());

        // SAFETY: config_new is the module's factory function. We
        // pass 0 as config_envoy_ptr since Praxis does not need
        // config-level callbacks from the module in the PoC.
        let config_ptr = unsafe { (http_symbols.config_new)(std::ptr::null_mut(), name_buf, config_buf) };

        if config_ptr.is_null() {
            return Err(format!("dynamic module '{}': config_new returned null", path.display()).into());
        }

        tracing::info!(
            module = %path.display(),
            filter_name = %name,
            "Envoy dynamic module HTTP filter configured"
        );

        Ok(Box::new(Self {
            module,
            config_ptr,
            filter_name: name,
        }))
    }
}

impl Drop for EnvoyDynamicModuleFilter {
    fn drop(&mut self) {
        if let Some(http) = &self.module.http {
            // SAFETY: config_ptr was returned by config_new and has
            // not been destroyed yet.
            unsafe { (http.config_destroy)(self.config_ptr) };
        }
    }
}

// ---------------------------------------------------------------------------
// HttpFilter Implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl HttpFilter for EnvoyDynamicModuleFilter {
    fn name(&self) -> &'static str {
        "envoy_dynamic_module"
    }

    #[allow(clippy::too_many_lines, reason = "FFI context setup + call + result mapping")]
    async fn on_request(&self, ctx: &mut HttpFilterContext<'_>) -> Result<FilterAction, praxis_filter::FilterError> {
        let http = self.module.http.as_ref().ok_or("no HTTP symbols")?;

        let Some(request_headers_fn) = http.request_headers else {
            return Ok(FilterAction::Continue);
        };

        let mut envoy_ctx = EnvoyHttpFilterContext {
            request_headers: &ctx.request.headers,
            request_method: &ctx.request.method,
            request_uri: &ctx.request.uri,
            extra_request_headers: &mut ctx.extra_request_headers,
            request_headers_to_remove: &mut ctx.request_headers_to_remove,
            request_headers_to_set: &mut ctx.request_headers_to_set,
            response_header: None,
            sent_response: None,
            #[cfg(debug_assertions)]
            poisoned: false,
        };

        let envoy_ctx_ptr: *mut EnvoyHttpFilterContext<'_> = &mut envoy_ctx;
        let envoy_ctx_raw = envoy_ctx_ptr as crate::abi::FilterEnvoyPtr;

        // Create per-request filter instance.
        // SAFETY: config_ptr is valid; envoy_ctx_raw points to our
        // stack-local context that outlives this call.
        let filter_module_ptr = unsafe { (http.filter_new)(self.config_ptr, envoy_ctx_raw) };

        if filter_module_ptr.is_null() {
            return Err(format!("dynamic module '{}': filter_new returned null", self.filter_name).into());
        }

        let _guard = FilterDestroyGuard {
            destroy_fn: http.filter_destroy,
            filter_ptr: filter_module_ptr,
        };

        // SAFETY: envoy_ctx_raw and filter_module_ptr are valid.
        // The call is synchronous — callbacks access envoy_ctx via
        // the pointer during this call only.
        let status = unsafe { request_headers_fn(envoy_ctx_raw, filter_module_ptr, true) };

        #[cfg(debug_assertions)]
        {
            envoy_ctx.poisoned = true;
        }

        if let Some(resp) = envoy_ctx.sent_response.take() {
            let mut rejection = Rejection::status(resp.status_code);
            for (k, v) in resp.headers {
                rejection = rejection.with_header(k, v);
            }
            if let Some(body) = resp.body {
                rejection = rejection.with_body(body);
            }
            return Ok(FilterAction::Reject(rejection));
        }

        if status != REQUEST_HEADERS_CONTINUE {
            tracing::trace!(
                filter = %self.filter_name,
                status,
                "Dynamic module returned non-Continue status (mapped to Continue)"
            );
        }

        Ok(FilterAction::Continue)
    }

    #[allow(clippy::too_many_lines, reason = "FFI context setup + call + result mapping")]
    async fn on_response(&self, ctx: &mut HttpFilterContext<'_>) -> Result<FilterAction, praxis_filter::FilterError> {
        let http = self.module.http.as_ref().ok_or("no HTTP symbols")?;

        let Some(response_headers_fn) = http.response_headers else {
            return Ok(FilterAction::Continue);
        };

        let mut envoy_ctx = EnvoyHttpFilterContext {
            request_headers: &ctx.request.headers,
            request_method: &ctx.request.method,
            request_uri: &ctx.request.uri,
            extra_request_headers: &mut ctx.extra_request_headers,
            request_headers_to_remove: &mut ctx.request_headers_to_remove,
            request_headers_to_set: &mut ctx.request_headers_to_set,
            response_header: ctx.response_header.as_deref_mut(),
            sent_response: None,
            #[cfg(debug_assertions)]
            poisoned: false,
        };

        let envoy_ctx_ptr: *mut EnvoyHttpFilterContext<'_> = &mut envoy_ctx;
        let envoy_ctx_raw = envoy_ctx_ptr as crate::abi::FilterEnvoyPtr;

        // SAFETY: same as on_request.
        let filter_module_ptr = unsafe { (http.filter_new)(self.config_ptr, envoy_ctx_raw) };

        if filter_module_ptr.is_null() {
            return Err(format!("dynamic module '{}': filter_new returned null", self.filter_name).into());
        }

        let _guard = FilterDestroyGuard {
            destroy_fn: http.filter_destroy,
            filter_ptr: filter_module_ptr,
        };

        // SAFETY: same as on_request.
        let status = unsafe { response_headers_fn(envoy_ctx_raw, filter_module_ptr, true) };

        #[cfg(debug_assertions)]
        {
            envoy_ctx.poisoned = true;
        }

        if let Some(resp) = envoy_ctx.sent_response.take() {
            let mut rejection = Rejection::status(resp.status_code);
            for (k, v) in resp.headers {
                rejection = rejection.with_header(k, v);
            }
            if let Some(body) = resp.body {
                rejection = rejection.with_body(body);
            }
            return Ok(FilterAction::Reject(rejection));
        }

        if status != RESPONSE_HEADERS_CONTINUE {
            tracing::trace!(
                filter = %self.filter_name,
                status,
                "Dynamic module returned non-Continue response status (mapped to Continue)"
            );
        }

        Ok(FilterAction::Continue)
    }
}
