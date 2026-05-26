// SPDX-License-Identifier: MIT

//! YAML configuration for the Envoy dynamic module filter.

use serde::Deserialize;

/// Configuration for the `envoy_dynamic_module` filter type.
///
/// # Example
///
/// ```yaml
/// - filter: envoy_dynamic_module
///   module_path: /usr/lib/praxis/modules/my_filter.so
///   module_name: my_filter
///   module_config: '{"key": "value"}'
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicModuleConfig {
    /// Filesystem path to the `.so` file.
    pub module_path: String,

    /// Logical name passed to the module's `config_new`. Defaults
    /// to the filename stem if omitted.
    #[serde(default)]
    pub module_name: Option<String>,

    /// Opaque configuration string forwarded to `config_new`.
    #[serde(default)]
    pub module_config: Option<String>,
}
