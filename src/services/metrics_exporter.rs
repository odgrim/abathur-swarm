//! Prometheus metrics exporter initialization.
//!
//! Installs a global `metrics` recorder backed by
//! `metrics-exporter-prometheus` and serves the Prometheus text-format on
//! `127.0.0.1:<port>/metrics`. The exporter is **optional** — if the port is
//! `0` or the configured env var is `disabled`, no recorder is installed and
//! all `metrics::counter!/gauge!/histogram!` calls in the codebase become
//! no-ops. Tests and non-CLI callers therefore do not need to call this
//! function.
//!
//! Configuration:
//! - `ABATHUR_METRICS_PORT` env var (default: `9091`). Set to `0` or
//!   `disabled` to skip exporter setup entirely.
//!
//! Labels throughout the codebase are intentionally cardinality-bounded —
//! only task type, outcome, and handler name strings are used. No user IDs,
//! task IDs, or goal IDs appear as labels.

use std::net::SocketAddr;

/// Default port when `ABATHUR_METRICS_PORT` is unset.
pub const DEFAULT_METRICS_PORT: u16 = 9091;

/// Env var that overrides the default metrics port. Set to `0` or `disabled`
/// to skip installing the exporter.
pub const METRICS_PORT_ENV: &str = "ABATHUR_METRICS_PORT";

/// Install the global Prometheus recorder and bind a `/metrics` HTTP
/// listener. Returns the bound address on success, or `None` if the exporter
/// is disabled. Failures to install are logged but non-fatal — the binary
/// continues without metrics.
pub fn install_from_env() -> Option<SocketAddr> {
    let raw = std::env::var(METRICS_PORT_ENV).ok();
    match raw.as_deref() {
        Some("disabled") | Some("off") | Some("false") => {
            tracing::info!(
                "metrics exporter disabled via {}={}",
                METRICS_PORT_ENV,
                raw.unwrap_or_default()
            );
            return None;
        }
        _ => {}
    }

    let port: u16 = raw
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_METRICS_PORT);

    if port == 0 {
        tracing::info!("metrics exporter disabled (port=0)");
        return None;
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    match metrics_exporter_prometheus::PrometheusBuilder::new()
        .with_http_listener(addr)
        .install()
    {
        Ok(()) => {
            tracing::info!(
                metrics_addr = %addr,
                "Prometheus metrics exporter listening on http://{}/metrics",
                addr
            );
            Some(addr)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                metrics_addr = %addr,
                "failed to install Prometheus metrics exporter; continuing without metrics"
            );
            None
        }
    }
}
