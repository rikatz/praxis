// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

use std::net::TcpStream;

use praxis_test_utils::{PraxisProcess, free_port, http_get, simple_proxy_yaml};

#[test]
#[ignore = "make test-integration runs this against a no-admin-api binary"]
fn health_and_metrics_run_without_admin_api() {
    let proxy_port = free_port();
    let admin_addr = format!("127.0.0.1:{}", free_port());
    let metrics_addr = format!("127.0.0.1:{}", free_port());
    let config = format!(
        "{}\nadmin:\n  address: \"{admin_addr}\"\n  metrics_address: \"{metrics_addr}\"\n",
        simple_proxy_yaml(proxy_port, free_port())
    );
    let _proxy = PraxisProcess::spawn(&config, &metrics_addr);

    assert!(
        TcpStream::connect(&admin_addr).is_err(),
        "admin listener should not bind without the admin-api feature"
    );
    assert_health_and_metrics(&metrics_addr);
    assert_api_routes_are_absent(&metrics_addr);
}

/// Assert that the always-available metrics listener serves its probe routes.
fn assert_health_and_metrics(metrics_addr: &str) {
    for path in ["/healthy", "/ready", "/metrics"] {
        let (status, body) = http_get(metrics_addr, path, None);
        assert_eq!(status, 200, "{path} should return 200: {body}");
    }
}

/// Assert that no Admin API route is exposed on the metrics listener.
fn assert_api_routes_are_absent(metrics_addr: &str) {
    for path in ["/api/stats", "/api/pipelines", "/api/kv/test", "/api/log-level"] {
        let (status, body) = http_get(metrics_addr, path, None);
        assert_eq!(status, 404, "metrics listener must not expose {path}: {body}");
    }
}
