//! Authentication (Option B: session-cookie auth; Option D hook included).
//!
//! `CurrentUser` is an axum `FromRequestParts` extractor — every protected
//! handler simply declares it as a parameter, and auth is enforced at the
//! extractor boundary (401 rejection). Write handlers additionally call
//! `require_admin(&user)?` for the admin/viewer split (Q3-b).
//!
//! The *source* of the user is decided by config:
//!
//! | Mode | Source | When |
//! |---|---|---|
//! | `SESSION` (default) | Redis session looked up from the `sentinel_session` cookie | local/team logins (B) |
//! | `TRUSTED_PROXY` (opt-in) | `X-Authenticated-User` header, accepted only from `TRUSTED_PROXY_IP` | behind a reverse proxy / IdP (D) |
//!
//! Both can be enabled at once. Enabling D later is a config flip — handlers,
//! the `users` table, and role guards are untouched.

pub mod password;
pub mod session;

use std::net::{IpAddr, SocketAddr};

use axum::extract::connect_info::ConnectInfo;
use axum::extract::FromRequestParts;
use axum::http::header::COOKIE;
use axum::http::{request::Parts, HeaderMap};

use crate::db::models::Role;
use crate::db::repo;
use crate::handlers::error::ApiError;
use crate::AppState;

/// The authenticated principal, extracted by `FromRequestParts`.
///
/// Add `user: CurrentUser` (or `_user: CurrentUser` for auth-only) to any
/// handler/WS-upgrade that requires a logged-in user.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: i64,
    pub email: String,
    pub role: Role,
    pub source: AuthSource,
}

/// How the current user was authenticated (for audit logging / future UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    /// Session cookie → Redis lookup (Option B).
    Session,
    /// Trusted reverse-proxy `X-Authenticated-User` header (Option D).
    TrustedProxy,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        resolve_user(
            state,
            &parts.headers,
            parts.extensions.get::<ConnectInfo<SocketAddr>>(),
        )
        .await
    }
}

/// Resolve the current user from the enabled source(s).
///
/// Order: trusted-proxy (if enabled + IP matches + header present) → session
/// cookie → 401.
async fn resolve_user(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<&ConnectInfo<SocketAddr>>,
) -> Result<CurrentUser, ApiError> {
    // 1. Trusted-proxy path (Option D), if enabled.
    if state.config.enable_trusted_proxy_auth {
        if let Some(proxy_ip) = state.config.trusted_proxy_ip.as_deref() {
            if let Some(ci) = peer {
                if peer_matches(ci.0, proxy_ip) {
                    if let Some(email) = headers
                        .get("x-authenticated-user")
                        .and_then(|v| v.to_str().ok())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        let user = repo::users::find_by_email(&state.db, email)
                            .await
                            .map_err(ApiError::from)?
                            .ok_or_else(|| {
                                ApiError::unauthorized("trusted-proxy user not provisioned")
                            })?;
                        return Ok(CurrentUser {
                            id: user.id,
                            email: user.email,
                            role: user.role,
                            source: AuthSource::TrustedProxy,
                        });
                    }
                }
            }
        }
    }

    // 2. Session-cookie path (Option B, default).
    let token = cookie_value(headers, &state.config.session_cookie_name)
        .ok_or_else(|| ApiError::unauthorized("no session"))?;
    let data = session::get(&state.redis, &token, state.config.session_ttl_secs)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e).context("session store error")))?
        .ok_or_else(|| ApiError::unauthorized("session expired"))?;
    Ok(CurrentUser {
        id: data.user_id,
        email: data.email,
        role: data.role,
        source: AuthSource::Session,
    })
}

/// Extract a cookie value from the `Cookie` request header by name.
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let header = headers.get(COOKIE)?.to_str().ok()?;
    for pair in header.split(';') {
        if let Some((k, v)) = pair.trim().split_once('=') {
            if k == name {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// True when the request's peer IP equals the configured trusted-proxy IP.
fn peer_matches(addr: SocketAddr, proxy_ip: &str) -> bool {
    match proxy_ip.parse::<IpAddr>() {
        Ok(ip) => addr.ip() == ip,
        Err(_) => {
            tracing::warn!(%proxy_ip, "TRUSTED_PROXY_IP is not a valid IP address");
            false
        }
    }
}
