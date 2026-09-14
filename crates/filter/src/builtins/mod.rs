// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2024 Praxis Contributors

//! Built-in filter implementations, organized by protocol and category.

pub mod http;
mod tcp;

#[cfg(feature = "basic-auth-filter")]
pub(crate) use http::BasicAuthConfig;
#[cfg(feature = "basic-auth-filter")]
pub use http::BasicAuthFilter;
#[cfg(feature = "policy-engine")]
pub(crate) use http::PolicyFilterConfig;
pub(crate) use http::{
    AccessLogConfig, CircuitBreakerConfig, CompressionFilterConfig, CorsConfig, CredentialInjectionConfig, CsrfConfig,
    EndpointSelectorConfig, ForwardedHeadersConfig, GrpcDetectionConfig, GuardrailsConfig, HeaderFilterConfig,
    IpAclConfig, IterativeRequestRouterConfig, JsonBodyFieldConfig, JsonRpcConfig, LoadBalancerConfig,
    PathRewriteConfig, PeerIdentityTrustConfig, RateLimitConfig, RequestIdFilterConfig, RouterConfig,
    StaticResponseConfig, StickySessionsConfig, TimeoutFilterConfig, TraceContextFilterConfig, UrlRewriteConfig,
    traffic_management::RedirectConfig,
};
pub use http::{
    AccessLogFilter, CircuitBreakerFilter, CompressionFilter, ContainsValue, CorsFilter, CredentialInjectionFilter,
    CsrfFilter, DisallowedOriginMode, EndpointReselector, EndpointSelectorFilter, ForwardedHeadersFilter,
    GrpcDetectionFilter, GuardrailsAction, GuardrailsFilter, HeaderFilter, IpAclFilter, IterativeRequestRouterFilter,
    JsonBodyFieldFilter, JsonRpcFilter, LoadBalancerFilter, PathRewriteFilter, PeerIdentityTrustFilter, PiiKind,
    RateLimitFilter, RateLimitMode, RedirectFilter, RedirectStatus, RequestIdFilter, RouterFilter, RuleTargetKind,
    SessionStore, SessionStoreRegistry, StaticResponseFilter, StickySessionsFilter, TimeoutFilter, TraceContextFilter,
    UrlRewriteFilter, access_record_already_emitted, bodyless_response, emit_access_record, has_dot_dot_traversal,
    mark_access_record_emitted, normalize_rewritten_path,
};
#[cfg(feature = "policy-engine")]
pub use http::{PolicyFilter, PolicyPluginFactoryFn, register_policy_plugin_factory, set_policy_subrequest_connector};
pub(crate) use tcp::{SniRouterConfig, TcpLoadBalancerConfig};
pub use tcp::{SniRouterFilter, TcpAccessLogFilter, TcpLoadBalancerFilter};
