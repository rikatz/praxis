// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2024 Praxis Contributors

//! HTTP observability filters: structured access logs, request correlation IDs,
//! and W3C Trace Context propagation.

mod access_log;
mod request_id;
mod trace_context;

pub(crate) use access_log::AccessLogConfig;
pub use access_log::{
    AccessLogFilter, access_record_already_emitted, bodyless_response, emit_access_record, mark_access_record_emitted,
};
pub use request_id::RequestIdFilter;
pub(crate) use request_id::RequestIdFilterConfig;
pub use trace_context::TraceContextFilter;
pub(crate) use trace_context::TraceContextFilterConfig;
