// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2024 Praxis Contributors

//! HTTP traffic management filters: routing, load balancing, timeout enforcement, redirects and static responses.

mod circuit_breaker;
mod endpoint_selector;
mod grpc_detection;
mod iterative_request_router;
mod load_balancer;
mod rate_limit;
mod redirect;
mod router;
mod static_response;
pub(crate) mod sticky_sessions;
mod timeout;
pub(crate) mod token_bucket;

pub(crate) use circuit_breaker::CircuitBreakerConfig;
pub use circuit_breaker::CircuitBreakerFilter;
pub(crate) use endpoint_selector::EndpointSelectorConfig;
pub use endpoint_selector::EndpointSelectorFilter;
pub(crate) use grpc_detection::GrpcDetectionConfig;
pub use grpc_detection::GrpcDetectionFilter;
pub(crate) use iterative_request_router::IterativeRequestRouterConfig;
pub use iterative_request_router::IterativeRequestRouterFilter;
pub(crate) use load_balancer::LoadBalancerConfig;
pub use load_balancer::{EndpointReselector, LoadBalancerFilter};
pub(crate) use rate_limit::RateLimitConfig;
pub use rate_limit::{RateLimitFilter, RateLimitMode};
pub(crate) use redirect::RedirectConfig;
pub use redirect::{RedirectFilter, RedirectStatus};
pub(crate) use router::RouterConfig;
pub use router::RouterFilter;
pub(crate) use static_response::StaticResponseConfig;
pub use static_response::StaticResponseFilter;
pub use sticky_sessions::StickySessionsFilter;
pub(crate) use sticky_sessions::config::StickySessionsConfig;
pub use timeout::TimeoutFilter;
pub(crate) use timeout::TimeoutFilterConfig;

// -----------------------------------------------------------------------------
// Utilities
// -----------------------------------------------------------------------------

/// Strip the port from a `Host` header value, handling bracketed IPv6.
///
/// ```ignore
/// assert_eq!(strip_port("example.com:8080"), "example.com");
/// assert_eq!(strip_port("[::1]:8080"), "[::1]");
/// assert_eq!(strip_port("example.com"), "example.com");
/// ```
pub(crate) fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        match host.find(']') {
            Some(i) => host.get(..=i).unwrap_or(host),
            None => host,
        }
    } else {
        host.split(':').next().unwrap_or(host)
    }
}
