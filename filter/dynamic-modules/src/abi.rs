// SPDX-License-Identifier: MIT

//! Safe Rust wrappers over the generated Envoy ABI types.
//!
//! Re-exports the generated types and adds ergonomic helpers
//! for buffer conversions and status enum interpretation.

pub(crate) use abi_sys::{
    envoy_dynamic_module_type_envoy_buffer as EnvoyBuffer,
    envoy_dynamic_module_type_envoy_http_header as EnvoyHttpHeader,
    envoy_dynamic_module_type_http_filter_config_envoy_ptr as ConfigEnvoyPtr,
    envoy_dynamic_module_type_http_filter_config_module_ptr as ConfigModulePtr,
    envoy_dynamic_module_type_http_filter_envoy_ptr as FilterEnvoyPtr,
    envoy_dynamic_module_type_http_filter_module_ptr as FilterModulePtr,
    envoy_dynamic_module_type_http_header_type_envoy_dynamic_module_type_http_header_type_RequestHeader as HEADER_TYPE_REQUEST,
    envoy_dynamic_module_type_http_header_type_envoy_dynamic_module_type_http_header_type_ResponseHeader as HEADER_TYPE_RESPONSE,
    envoy_dynamic_module_type_module_buffer as ModuleBuffer,
    envoy_dynamic_module_type_module_http_header as ModuleHttpHeader,
    envoy_dynamic_module_type_on_http_filter_request_headers_status as RequestHeadersStatus,
    envoy_dynamic_module_type_on_http_filter_request_headers_status_envoy_dynamic_module_type_on_http_filter_request_headers_status_Continue as REQUEST_HEADERS_CONTINUE,
    envoy_dynamic_module_type_on_http_filter_response_headers_status as ResponseHeadersStatus,
    envoy_dynamic_module_type_on_http_filter_response_headers_status_envoy_dynamic_module_type_on_http_filter_response_headers_status_Continue as RESPONSE_HEADERS_CONTINUE,
};

use crate::abi_sys;

// ---------------------------------------------------------------------------
// Buffer Helpers
// ---------------------------------------------------------------------------

/// Create an [`EnvoyBuffer`] pointing to a byte slice.
///
/// The returned buffer borrows the slice — the caller must ensure the
/// slice outlives the buffer.
pub(crate) fn envoy_buffer_from_slice(s: &[u8]) -> EnvoyBuffer {
    EnvoyBuffer {
        ptr: s.as_ptr().cast::<std::os::raw::c_char>(),
        length: s.len(),
    }
}

/// Create a [`ModuleBuffer`] pointing to a byte slice.
#[allow(dead_code, reason = "used by future ABI callbacks")]
pub(crate) fn module_buffer_from_slice(s: &[u8]) -> ModuleBuffer {
    ModuleBuffer {
        ptr: s.as_ptr().cast::<std::os::raw::c_char>(),
        length: s.len(),
    }
}

/// Interpret an [`EnvoyBuffer`] as a byte slice.
///
/// # Safety
///
/// The buffer's `ptr` must be valid for `length` bytes, or `ptr` must
/// be null (in which case an empty slice is returned).
pub(crate) unsafe fn envoy_buffer_as_slice(buf: &EnvoyBuffer) -> &[u8] {
    if buf.ptr.is_null() || buf.length == 0 {
        return &[];
    }
    // SAFETY: caller guarantees ptr is valid for length bytes.
    unsafe { std::slice::from_raw_parts(buf.ptr.cast::<u8>(), buf.length) }
}

/// Interpret a [`ModuleBuffer`] as a byte slice.
///
/// # Safety
///
/// The buffer's `ptr` must be valid for `length` bytes, or `ptr` must
/// be null (in which case an empty slice is returned).
pub(crate) unsafe fn module_buffer_as_slice(buf: &ModuleBuffer) -> &[u8] {
    if buf.ptr.is_null() || buf.length == 0 {
        return &[];
    }
    // SAFETY: caller guarantees ptr is valid for length bytes.
    unsafe { std::slice::from_raw_parts(buf.ptr.cast::<u8>(), buf.length) }
}
