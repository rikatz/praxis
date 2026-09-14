// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2024 Praxis Contributors

//! HTTP transformation filters: header manipulation, path rewriting, and URL rewriting.

mod header;
mod path_rewrite;
pub(crate) mod path_sanitize;
mod url_rewrite;

pub use header::HeaderFilter;
pub(crate) use header::HeaderFilterConfig;
pub(crate) use path_rewrite::PathRewriteConfig;
pub use path_rewrite::PathRewriteFilter;
pub use path_sanitize::{has_dot_dot_traversal, normalize_rewritten_path};
pub(crate) use url_rewrite::UrlRewriteConfig;
pub use url_rewrite::UrlRewriteFilter;
