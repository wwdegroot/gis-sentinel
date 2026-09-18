//! HTTP probe transport (task 2.1: on-demand `/test` endpoint).
//!
//! Measures high-resolution latency via [`std::time::Instant`] and captures
//! the HTTP status, an error message, and a response snippet. GIS-level
//! validation (WMS/WFS GetCapabilities, ArcGIS `f=pjson`, OAF landing page)
//! is layered on top in task 2.3 — this module only answers "did the
//! transport work, and how fast?".

use axum::http::HeaderMap;
use reqwest::Method;
use std::time::{Duration, Instant};
use tracing::debug;

/// Maximum characters captured from the response body for diagnostics.
const SNIPPET_MAX_CHARS: usize = 2000;

/// Result of a single probe execution (transport level).
///
/// Mirrors the persistable columns of `probe_results` so the on-demand test
/// endpoint can echo the exact shape that the worker (task 2.3) will persist.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProbeObservation {
    /// HTTP status code, `None` when the request never got a response.
    pub status_code: Option<i32>,
    /// Total transfer duration in milliseconds (headers + body), `None` on transport failure.
    pub response_time_ms: Option<i32>,
    /// `true` when the probe succeeded at the transport level (2xx/3xx).
    pub is_up: bool,
    /// Human-readable failure reason (connection error, timeout, ...).
    pub error_message: Option<String>,
    /// First [`SNIPPET_MAX_CHARS`] characters of the response body.
    pub raw_response_snippet: Option<String>,
}

/// Execute a single HTTP probe against `url`.
///
/// * `method` — GET or POST (validated upstream).
/// * `headers` — custom request headers from the alert point configuration.
/// * `timeout` — overall request timeout (connection + response + body read).
///
/// Transport-level errors (DNS, connect, timeout, TLS) yield `is_up = false`
/// with a populated `error_message`; HTTP 4xx/5xx also count as down.
pub async fn run_http_probe(
    client: &reqwest::Client,
    url: &str,
    method: Method,
    headers: &HeaderMap,
    timeout: Duration,
) -> ProbeObservation {
    let started = Instant::now();

    let mut request = client.request(method, url).timeout(timeout);
    for (name, value) in headers.iter() {
        request = request.header(name, value);
    }

    let response = match request.send().await {
        Ok(resp) => resp,
        Err(e) => {
            let msg = describe_reqwest_error(&e);
            debug!(url, error = %msg, "probe transport failure");
            return ProbeObservation {
                status_code: None,
                response_time_ms: None,
                is_up: false,
                error_message: Some(msg),
                raw_response_snippet: None,
            };
        }
    };

    // Headers received — elapsed() so far approximates TTFB.
    let ttfb = started.elapsed();
    debug!(
        url,
        ttfb_ms = ttfb.as_millis() as i64,
        "probe headers received"
    );

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let total_ms = started.elapsed();

    let snippet: String = body.chars().take(SNIPPET_MAX_CHARS).collect();
    let is_up = status.is_success() || status.is_redirection();

    ProbeObservation {
        status_code: Some(status.as_u16() as i32),
        response_time_ms: Some(i32::try_from(total_ms.as_millis()).unwrap_or(i32::MAX)),
        is_up,
        error_message: if is_up {
            None
        } else {
            Some(format!("HTTP {status}"))
        },
        raw_response_snippet: (!snippet.is_empty()).then_some(snippet),
    }
}

/// Collapse a reqwest error into a short human-readable message.
pub fn describe_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "request timed out".to_string()
    } else if e.is_connect() {
        format!("connection failed: {e}")
    } else if let Some(url) = e.url() {
        format!("request to {url} failed: {e}")
    } else {
        e.to_string()
    }
}

/// Build the shared probe [`reqwest::Client`] (connection pooling, no system
/// proxy surprises, sane defaults for a monitoring workload).
pub fn build_probe_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("gis-sentinel/", env!("CARGO_PKG_VERSION")))
        // Safety net; the per-request timeout from the job is authoritative.
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("static reqwest client configuration is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_host_yields_down_observation() {
        // Reserved documentation IP: nothing to connect to, fails fast.
        let client = reqwest::Client::new();
        let obs = run_http_probe(
            &client,
            "http://127.0.0.1:1/",
            Method::GET,
            &HeaderMap::new(),
            Duration::from_secs(2),
        )
        .await;
        assert!(!obs.is_up);
        assert!(obs.status_code.is_none());
        assert!(obs.error_message.is_some());
    }
}
