// SPDX-License-Identifier: MIT

//! Per-invocation context for HTTP filter callbacks.
//!
//! An [`EnvoyHttpFilterContext`] is created on the stack inside each
//! `on_request` / `on_response` call. A raw pointer to it is passed
//! as the `filter_envoy_ptr` to the dynamic module. When the module
//! calls back into the host (e.g. `callback_http_get_header`), the
//! callback casts the pointer back to access Praxis request/response
//! data.

use std::borrow::Cow;

use bytes::Bytes;
use http::header::{HeaderName, HeaderValue};
use praxis_filter::Response;

// ---------------------------------------------------------------------------
// EnvoyHttpFilterContext
// ---------------------------------------------------------------------------

/// Praxis-side state exposed to an Envoy dynamic module via the
/// `filter_envoy_ptr` opaque pointer.
///
/// # Lifetime
///
/// The context borrows from the [`HttpFilterContext`] and lives on
/// the stack for exactly one synchronous C call. The pointer becomes
/// invalid the instant the call returns.
///
/// [`HttpFilterContext`]: praxis_filter::HttpFilterContext
#[allow(dead_code, reason = "fields used by future ABI callbacks")]
pub(crate) struct EnvoyHttpFilterContext<'a> {
    /// Request headers (read-only from the module's perspective;
    /// mutations go through the `_to_set` / `_to_remove` vectors).
    pub(crate) request_headers: &'a http::HeaderMap,

    /// Request method.
    pub(crate) request_method: &'a http::Method,

    /// Request URI.
    pub(crate) request_uri: &'a http::Uri,

    /// Headers to add to the upstream request.
    pub(crate) extra_request_headers: &'a mut Vec<(Cow<'static, str>, String)>,

    /// Headers to remove from the upstream request.
    pub(crate) request_headers_to_remove: &'a mut Vec<HeaderName>,

    /// Headers to overwrite on the upstream request.
    pub(crate) request_headers_to_set: &'a mut Vec<(HeaderName, HeaderValue)>,

    /// Mutable response headers (available only during response phase).
    pub(crate) response_header: Option<&'a mut Response>,

    /// Populated by [`callback_http_send_response`] if the module
    /// sends a local response.
    ///
    /// [`callback_http_send_response`]: super::callbacks
    pub(crate) sent_response: Option<SentResponse>,

    /// Debug-mode liveness flag. Set to `true` after the hook returns;
    /// callbacks check this and panic if the pointer is used after
    /// invalidation.
    #[cfg(debug_assertions)]
    pub(crate) poisoned: bool,
}

// ---------------------------------------------------------------------------
// SentResponse
// ---------------------------------------------------------------------------

/// Captures a local response sent by the module via
/// `envoy_dynamic_module_callback_http_send_response`.
pub(crate) struct SentResponse {
    /// HTTP status code.
    pub(crate) status_code: u16,
    /// Response headers.
    pub(crate) headers: Vec<(String, String)>,
    /// Optional response body.
    pub(crate) body: Option<Bytes>,
}
