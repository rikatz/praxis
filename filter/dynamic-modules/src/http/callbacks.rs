// SPDX-License-Identifier: MIT

//! Host-side callback implementations for the Envoy HTTP filter ABI.
//!
//! These `#[unsafe(no_mangle)] extern "C"` functions are resolved by the
//! dynamic module's `.so` at load time. Each receives the
//! `filter_envoy_ptr` which points to an [`EnvoyHttpFilterContext`]
//! on the Praxis stack.
//!
//! # Safety
//!
//! Every callback requires `filter_envoy_ptr` to be a valid pointer
//! to an [`EnvoyHttpFilterContext`] that outlives the call. This
//! invariant is maintained by the bridge filter: the context lives
//! on the stack for the duration of the synchronous C call.
//!
//! [`EnvoyHttpFilterContext`]: super::context::EnvoyHttpFilterContext

use super::context::{EnvoyHttpFilterContext, SentResponse};
use crate::{
    abi::{EnvoyBuffer, EnvoyHttpHeader, HEADER_TYPE_REQUEST, HEADER_TYPE_RESPONSE, ModuleBuffer, ModuleHttpHeader},
    abi_sys::{
        envoy_dynamic_module_type_http_filter_config_envoy_ptr, envoy_dynamic_module_type_http_filter_envoy_ptr,
        envoy_dynamic_module_type_http_header_type, envoy_dynamic_module_type_log_level,
    },
};

// ---------------------------------------------------------------------------
// Internal Helpers
// ---------------------------------------------------------------------------

/// Cast `filter_envoy_ptr` to a shared reference to the context.
///
/// # Safety
///
/// Caller must ensure the pointer is valid and not poisoned.
unsafe fn ctx_ref<'a>(ptr: envoy_dynamic_module_type_http_filter_envoy_ptr) -> &'a EnvoyHttpFilterContext<'a> {
    debug_assert!(!ptr.is_null(), "filter_envoy_ptr is null");
    // SAFETY: caller guarantees the pointer is valid.
    let ctx = unsafe { &*(ptr as *const EnvoyHttpFilterContext<'_>) };
    #[cfg(debug_assertions)]
    debug_assert!(!ctx.poisoned, "callback invoked after hook returned");
    ctx
}

/// Cast `filter_envoy_ptr` to a mutable reference to the context.
///
/// # Safety
///
/// Caller must ensure the pointer is valid, not poisoned, and that
/// no other reference exists.
unsafe fn ctx_mut<'a>(ptr: envoy_dynamic_module_type_http_filter_envoy_ptr) -> &'a mut EnvoyHttpFilterContext<'a> {
    debug_assert!(!ptr.is_null(), "filter_envoy_ptr is null");
    // SAFETY: caller guarantees exclusive access.
    let ctx = unsafe { &mut *ptr.cast::<EnvoyHttpFilterContext<'_>>() };
    #[cfg(debug_assertions)]
    debug_assert!(!ctx.poisoned, "callback invoked after hook returned");
    ctx
}

/// Resolve the header map for the given header type.
fn resolve_headers<'a>(
    ctx: &'a EnvoyHttpFilterContext<'_>,
    header_type: envoy_dynamic_module_type_http_header_type,
) -> Option<&'a http::HeaderMap> {
    if header_type == HEADER_TYPE_REQUEST {
        Some(ctx.request_headers)
    } else if header_type == HEADER_TYPE_RESPONSE {
        ctx.response_header.as_ref().map(|r| &r.headers)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Callback: get_header
// ---------------------------------------------------------------------------

/// Read a header value by key, supporting multi-value iteration.
///
/// When `index` is 0 and `optional_size` is non-null, writes the total
/// count of values for the key. The `index` parameter selects which
/// value to return (0-based).
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
/// `result_buffer` must point to a writable [`EnvoyBuffer`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_get_header(
    filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    header_type: envoy_dynamic_module_type_http_header_type,
    key: ModuleBuffer,
    result_buffer: *mut EnvoyBuffer,
    index: usize,
    optional_size: *mut usize,
) -> bool {
    // SAFETY: caller guarantees filter_envoy_ptr is valid.
    let ctx = unsafe { ctx_ref(filter_envoy_ptr) };

    let Some(headers) = resolve_headers(ctx, header_type) else {
        return false;
    };

    // SAFETY: the module buffer is valid for the duration of this call.
    let key_slice = unsafe { crate::abi::module_buffer_as_slice(&key) };
    let Ok(key_str) = std::str::from_utf8(key_slice) else {
        return false;
    };

    let all_values = headers.get_all(key_str);

    if !optional_size.is_null() {
        // SAFETY: optional_size is writable per caller contract.
        unsafe { *optional_size = all_values.iter().count() };
    }

    let Some(value) = all_values.iter().nth(index) else {
        return false;
    };

    if !result_buffer.is_null() {
        // SAFETY: result_buffer is writable per caller contract.
        let result = unsafe { &mut *result_buffer };
        result.ptr = value.as_bytes().as_ptr().cast::<std::os::raw::c_char>();
        result.length = value.len();
    }

    true
}

// ---------------------------------------------------------------------------
// Callback: get_headers_size
// ---------------------------------------------------------------------------

/// Return the number of headers of the given type.
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_get_headers_size(
    filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    header_type: envoy_dynamic_module_type_http_header_type,
) -> usize {
    // SAFETY: caller guarantees filter_envoy_ptr is valid.
    let ctx = unsafe { ctx_ref(filter_envoy_ptr) };
    resolve_headers(ctx, header_type).map_or(0, http::HeaderMap::len)
}

// ---------------------------------------------------------------------------
// Callback: get_headers
// ---------------------------------------------------------------------------

/// Fill an array of [`EnvoyHttpHeader`] with all headers of the given type.
///
/// The caller must pre-allocate the array with at least
/// `get_headers_size()` entries.
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
/// `result_headers` must point to a writable array with enough entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_get_headers(
    filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    header_type: envoy_dynamic_module_type_http_header_type,
    result_headers: *mut EnvoyHttpHeader,
) -> bool {
    // SAFETY: caller guarantees filter_envoy_ptr is valid.
    let ctx = unsafe { ctx_ref(filter_envoy_ptr) };

    let Some(headers) = resolve_headers(ctx, header_type) else {
        return false;
    };

    if result_headers.is_null() {
        return false;
    }

    for (i, (name, value)) in headers.iter().enumerate() {
        // SAFETY: result_headers has at least headers.len() entries.
        let entry = unsafe { &mut *result_headers.add(i) };
        entry.key_ptr = name.as_str().as_ptr().cast::<std::os::raw::c_char>();
        entry.key_length = name.as_str().len();
        entry.value_ptr = value.as_bytes().as_ptr().cast::<std::os::raw::c_char>();
        entry.value_length = value.len();
    }

    true
}

// ---------------------------------------------------------------------------
// Callback: set_header
// ---------------------------------------------------------------------------

/// Set (overwrite) a header value.
///
/// For request headers, pushes to `request_headers_to_set`. For
/// response headers, directly mutates the response `HeaderMap`.
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
#[allow(clippy::too_many_lines, reason = "request + response header branching")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_set_header(
    filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    header_type: envoy_dynamic_module_type_http_header_type,
    key: ModuleBuffer,
    value: ModuleBuffer,
) -> bool {
    // SAFETY: caller guarantees filter_envoy_ptr is valid.
    let ctx = unsafe { ctx_mut(filter_envoy_ptr) };

    // SAFETY: module buffers are valid for the duration of this call.
    let key_slice = unsafe { crate::abi::module_buffer_as_slice(&key) };
    // SAFETY: module buffers are valid for the duration of this call.
    let value_slice = unsafe { crate::abi::module_buffer_as_slice(&value) };

    let Ok(key_str) = std::str::from_utf8(key_slice) else {
        return false;
    };

    if header_type == HEADER_TYPE_REQUEST {
        let Ok(name) = http::header::HeaderName::from_bytes(key_slice) else {
            return false;
        };
        let Ok(val) = http::header::HeaderValue::from_bytes(value_slice) else {
            return false;
        };
        ctx.request_headers_to_set.push((name, val));
        true
    } else if header_type == HEADER_TYPE_RESPONSE {
        let Some(resp) = ctx.response_header.as_mut() else {
            return false;
        };
        let Ok(name) = http::header::HeaderName::from_bytes(key_slice) else {
            return false;
        };
        let Ok(val) = http::header::HeaderValue::from_bytes(value_slice) else {
            return false;
        };
        resp.headers.insert(name, val);
        true
    } else {
        let _ = key_str;
        false
    }
}

// ---------------------------------------------------------------------------
// Callback: add_header
// ---------------------------------------------------------------------------

/// Add a header (does not remove existing values with the same name).
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_add_header(
    filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    header_type: envoy_dynamic_module_type_http_header_type,
    key: ModuleBuffer,
    value: ModuleBuffer,
) -> bool {
    // SAFETY: caller guarantees filter_envoy_ptr is valid.
    let ctx = unsafe { ctx_mut(filter_envoy_ptr) };

    // SAFETY: module buffers are valid for the duration of this call.
    let key_slice = unsafe { crate::abi::module_buffer_as_slice(&key) };
    // SAFETY: module buffers are valid for the duration of this call.
    let value_slice = unsafe { crate::abi::module_buffer_as_slice(&value) };

    if header_type == HEADER_TYPE_REQUEST {
        let Ok(key_str) = std::str::from_utf8(key_slice) else {
            return false;
        };
        let Ok(val_str) = std::str::from_utf8(value_slice) else {
            return false;
        };
        ctx.extra_request_headers
            .push((key_str.to_owned().into(), val_str.to_owned()));
        true
    } else if header_type == HEADER_TYPE_RESPONSE {
        let Some(resp) = ctx.response_header.as_mut() else {
            return false;
        };
        let Ok(name) = http::header::HeaderName::from_bytes(key_slice) else {
            return false;
        };
        let Ok(val) = http::header::HeaderValue::from_bytes(value_slice) else {
            return false;
        };
        resp.headers.append(name, val);
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Callback: send_response
// ---------------------------------------------------------------------------

/// Send a local response (short-circuit the filter chain).
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
#[allow(clippy::too_many_lines, reason = "header vector + body extraction from FFI")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_send_response(
    filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    status_code: u32,
    headers_vector: *const ModuleHttpHeader,
    headers_vector_size: usize,
    body: ModuleBuffer,
    _details: ModuleBuffer,
) {
    // SAFETY: caller guarantees filter_envoy_ptr is valid.
    let ctx = unsafe { ctx_mut(filter_envoy_ptr) };

    let mut response_headers = Vec::with_capacity(headers_vector_size);
    for i in 0..headers_vector_size {
        // SAFETY: headers_vector has headers_vector_size entries.
        let hdr = unsafe { &*headers_vector.add(i) };
        let key_buf = ModuleBuffer {
            ptr: hdr.key_ptr,
            length: hdr.key_length,
        };
        let val_buf = ModuleBuffer {
            ptr: hdr.value_ptr,
            length: hdr.value_length,
        };
        // SAFETY: module buffers point into the header vector, valid for this call.
        let key_slice = unsafe { crate::abi::module_buffer_as_slice(&key_buf) };
        // SAFETY: module buffers point into the header vector, valid for this call.
        let val_slice = unsafe { crate::abi::module_buffer_as_slice(&val_buf) };
        let key_str = String::from_utf8_lossy(key_slice).into_owned();
        let val_str = String::from_utf8_lossy(val_slice).into_owned();
        response_headers.push((key_str, val_str));
    }

    // SAFETY: module buffer is valid for this call.
    let body_slice = unsafe { crate::abi::module_buffer_as_slice(&body) };
    let body_bytes = if body_slice.is_empty() {
        None
    } else {
        Some(bytes::Bytes::copy_from_slice(body_slice))
    };

    #[allow(clippy::cast_possible_truncation, reason = "HTTP status codes fit in u16")]
    let status = status_code as u16;

    ctx.sent_response = Some(SentResponse {
        status_code: status,
        headers: response_headers,
        body: body_bytes,
    });
}

// ---------------------------------------------------------------------------
// Stub Callbacks (not yet implemented)
// ---------------------------------------------------------------------------

/// Stub: body size is always 0 in this implementation.
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_get_body_size(
    _filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    _body_type: u32,
) -> usize {
    0
}

/// Stub: body chunks retrieval is not implemented.
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_get_body_chunks(
    _filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    _body_type: u32,
    _result_buffer_vector: *mut EnvoyBuffer,
) -> bool {
    false
}

/// Stub: body chunks count is always 0.
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_get_body_chunks_size(
    _filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    _body_type: u32,
) -> usize {
    0
}

/// Stub: route module logging to `tracing`.
///
/// # Safety
///
/// Buffers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_log(
    level: envoy_dynamic_module_type_log_level,
    message: EnvoyBuffer,
) {
    // SAFETY: message buffer is valid for this call.
    let msg_slice = unsafe { crate::abi::envoy_buffer_as_slice(&message) };
    let msg = String::from_utf8_lossy(msg_slice);

    match level {
        0 => tracing::trace!(target: "envoy_dynamic_module", "{msg}"),
        1 => tracing::debug!(target: "envoy_dynamic_module", "{msg}"),
        2 => tracing::info!(target: "envoy_dynamic_module", "{msg}"),
        3 => tracing::warn!(target: "envoy_dynamic_module", "{msg}"),
        4 | 5 => tracing::error!(target: "envoy_dynamic_module", "{msg}"),
        _ => tracing::trace!(target: "envoy_dynamic_module", "{msg}"),
    }
}

/// Stub: log level check.
///
/// # Safety
///
/// No pointer arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_log_enabled(
    _level: envoy_dynamic_module_type_log_level,
) -> bool {
    true
}

/// Stub: concurrency (worker thread count). Returns 1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_get_concurrency() -> u32 {
    1
}

/// Stub: validation mode check. Always false.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_is_validation_mode() -> bool {
    false
}

/// Stub: function registry — register. Always returns false.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_register_function(
    _key: ModuleBuffer,
    _function: *const std::os::raw::c_void,
) -> bool {
    false
}

/// Stub: function registry — get. Always returns false.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_get_function(_key: ModuleBuffer) -> bool {
    false
}

/// Stub: shared data registry — register. Always returns false.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_register_shared_data(
    _key: ModuleBuffer,
    _data: *const std::os::raw::c_void,
) -> bool {
    false
}

/// Stub: shared data registry — get. Always returns false.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_get_shared_data(_key: ModuleBuffer) -> bool {
    false
}

/// Stub: config-level HTTP callout done — no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_filter_config_http_callout(
    _config_envoy_ptr: envoy_dynamic_module_type_http_filter_config_envoy_ptr,
    _callout_id: *mut u64,
    _cluster_name: ModuleBuffer,
    _headers: *const ModuleHttpHeader,
    _headers_size: usize,
    _body: ModuleBuffer,
    _timeout_ms: u32,
) -> u32 {
    4
}

// ---------------------------------------------------------------------------
// Dynamic Metadata Stubs
// ---------------------------------------------------------------------------

/// Stub: set string dynamic metadata. No-op in Praxis (no metadata store).
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_set_dynamic_metadata_string(
    _filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    _ns: ModuleBuffer,
    _key: ModuleBuffer,
    _value: ModuleBuffer,
) {
    tracing::trace!("set_dynamic_metadata_string: not supported in Praxis (no-op)");
}

/// Stub: set numeric dynamic metadata. No-op in Praxis.
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_set_dynamic_metadata_number(
    _filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    _ns: ModuleBuffer,
    _key: ModuleBuffer,
    _value: f64,
) {
    tracing::trace!("set_dynamic_metadata_number: not supported in Praxis (no-op)");
}

/// Stub: set boolean dynamic metadata. No-op in Praxis.
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_http_set_dynamic_metadata_bool(
    _filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
    _ns: ModuleBuffer,
    _key: ModuleBuffer,
    _value: bool,
) {
    tracing::trace!("set_dynamic_metadata_bool: not supported in Praxis (no-op)");
}

// ---------------------------------------------------------------------------
// Per-Route Config Stub
// ---------------------------------------------------------------------------

/// Stub: retrieve per-route config. Always returns null (not supported).
///
/// # Safety
///
/// `filter_envoy_ptr` must point to a valid [`EnvoyHttpFilterContext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn envoy_dynamic_module_callback_get_most_specific_route_config(
    _filter_envoy_ptr: envoy_dynamic_module_type_http_filter_envoy_ptr,
) -> crate::abi_sys::envoy_dynamic_module_type_http_filter_per_route_config_module_ptr {
    std::ptr::null()
}
