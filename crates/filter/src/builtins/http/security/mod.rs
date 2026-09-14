// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2024 Praxis Contributors

//! HTTP security filters: CORS, CSRF, IP access control, credential injection,
//! forwarded-header injection, guardrails, mTLS ingress trust enforcement,
//! and the (feature-gated) `policy` filter.

#[cfg(feature = "basic-auth-filter")]
mod basic_auth;
mod cors;
mod credential_injection;
mod csrf;
mod forwarded_headers;
mod guardrails;
mod ip_acl;
pub(crate) mod origin_matcher;
pub(crate) mod origin_normalize;
mod peer_identity_trust;
#[cfg(feature = "policy-engine")]
mod policy;

#[cfg(feature = "basic-auth-filter")]
pub(crate) use basic_auth::BasicAuthConfig;
#[cfg(feature = "basic-auth-filter")]
pub use basic_auth::BasicAuthFilter;
pub(crate) use cors::CorsConfig;
pub use cors::{CorsFilter, DisallowedOriginMode};
pub(crate) use credential_injection::CredentialInjectionConfig;
pub use credential_injection::CredentialInjectionFilter;
pub(crate) use csrf::CsrfConfig;
pub use csrf::CsrfFilter;
pub(crate) use forwarded_headers::ForwardedHeadersConfig;
pub use forwarded_headers::ForwardedHeadersFilter;
pub(crate) use guardrails::GuardrailsConfig;
pub use guardrails::{ContainsValue, GuardrailsAction, GuardrailsFilter, PiiKind, RuleTargetKind};
pub(crate) use ip_acl::IpAclConfig;
pub use ip_acl::IpAclFilter;
pub(crate) use peer_identity_trust::PeerIdentityTrustConfig;
pub use peer_identity_trust::PeerIdentityTrustFilter;
#[cfg(feature = "policy-engine")]
pub(crate) use policy::PolicyFilterConfig;
#[cfg(feature = "policy-engine")]
pub use policy::{
    PolicyFilter, PolicyPluginFactoryFn, register_policy_plugin_factory, set_policy_subrequest_connector,
};
