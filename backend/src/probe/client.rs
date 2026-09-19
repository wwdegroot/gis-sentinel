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
use tracing::{debug, warn};

use crate::queue::{AuthSpec, ProbeJob};

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
    auth: Option<&AuthSpec>,
) -> ProbeObservation {
    let started = Instant::now();

    let mut request = client.request(method, url).timeout(timeout);
    for (name, value) in headers.iter() {
        request = request.header(name, value);
    }
    if let Some(auth) = auth {
        request = apply_auth(request, auth);
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

/// Execute a full probe for a [`ProbeJob`] (task 2.3 worker path).
///
/// Applies the GIS-specific URL transformation (see [`super::gis`]), the
/// configured HTTP method, custom headers, auth, and per-job timeout, then
/// runs the transport probe. Service-level checking of the returned body is
/// a separate step ([`super::gis::check_service_response`]) so the caller
/// controls evaluation.
pub async fn run_job_probe(client: &reqwest::Client, job: &ProbeJob) -> ProbeObservation {
    let url = super::gis::build_probe_url(job);
    let method = match job.http_method {
        crate::db::models::HttpMethod::Get => Method::GET,
        crate::db::models::HttpMethod::Post => Method::POST,
    };

    let mut headers = HeaderMap::new();
    for (name, value) in &job.custom_headers {
        match (
            axum::http::HeaderName::from_lowercase(name.as_bytes()),
            axum::http::HeaderValue::from_str(value),
        ) {
            (Ok(n), Ok(v)) => {
                headers.insert(n, v);
            }
            _ => {
                warn!(target_id = %job.target_id, header = %name, "skipping invalid custom header")
            }
        }
    }

    let observation = run_http_probe(
        client,
        &url,
        method,
        &headers,
        Duration::from_millis(job.timeout_ms.max(1) as u64),
        job.auth.as_ref(),
    )
    .await;

    if job.auth.is_some() {
        debug!(target_id = %job.target_id, "probe sent with configured auth");
    }

    observation
}

/// Convert the REST API's `auth_config` JSONB into an [`AuthSpec`] for the
/// job payload. Returns `None` for absent or malformed configs (malformation
/// is logged by the scheduler; the API validates shapes upstream).
pub fn parse_auth_spec(auth_config: Option<&serde_json::Value>) -> Option<AuthSpec> {
    auth_config.and_then(|v| match serde_json::from_value::<AuthSpec>(v.clone()) {
        Ok(spec) => Some(spec),
        Err(e) => {
            warn!(error = %e, "failed to parse auth_config; probing without auth");
            None
        }
    })
}

/// Apply an [`AuthSpec`] to a request builder (helper for tests and the
/// on-demand endpoint; [`run_http_probe`] currently applies headers only).
pub fn apply_auth(builder: reqwest::RequestBuilder, auth: &AuthSpec) -> reqwest::RequestBuilder {
    match auth {
        AuthSpec::Basic { username, password } => builder.basic_auth(username, Some(password)),
        AuthSpec::Bearer { token } => builder.bearer_auth(token),
        AuthSpec::Header { name, value } => builder.header(name, value),
    }
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
            None,
        )
        .await;
        assert!(!obs.is_up);
        assert!(obs.status_code.is_none());
        assert!(obs.error_message.is_some());
    }
}
