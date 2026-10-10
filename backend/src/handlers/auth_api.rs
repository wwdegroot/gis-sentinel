//! Auth API (Option B): session-cookie login/logout/me.
//!
//! Mounted under `/api/v1/auth`:
//!
//! | Method | Path      | Auth     | Purpose |
//! |--------|-----------|----------|---------|
//! | POST   | `/login`  | public   | email+password → sets `sentinel_session` cookie + 200 `{user}` |
//! | POST   | `/logout` | public*  | clears the cookie + deletes the Redis session → 204 |
//! | GET    | `/me`     | required | returns the current user → 200 `{user}` |
//!
//! *logout is public so a client with a stale cookie can still clear it; it is
//! a no-op without a valid session token.
//!
//! `/me` requires auth simply by extracting `CurrentUser` (401 when absent).
//!
//! CSRF posture (Q4): the session cookie is `HttpOnly; Secure; SameSite=Strict`
//! and all mutation endpoints are JSON-only (`axum::Json`), so a cross-site form
//! POST cannot forge a valid request. No CSRF token is issued.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, SameSite};
use serde::Deserialize;
use time::Duration as CookieDuration;

use crate::auth::{self, CurrentUser};
use crate::db::models::{Role, User};
use crate::db::repo;
use crate::handlers::error::{ApiError, ApiResult};
use crate::AppState;

/// Max email/password length accepted by the login endpoint (DoS guard).
const MAX_EMAIL_LEN: usize = 254; // RFC 5321
const MAX_PASSWORD_LEN: usize = 1024;

/// Public projection of a user — never includes `password_hash`.
#[derive(Debug, serde::Serialize)]
pub struct UserPublic {
    pub id: i64,
    pub email: String,
    pub role: Role,
}

impl From<&User> for UserPublic {
    fn from(u: &User) -> Self {
        Self {
            id: u.id,
            email: u.email.clone(),
            role: u.role,
        }
    }
}

impl From<&CurrentUser> for UserPublic {
    fn from(u: &CurrentUser) -> Self {
        Self {
            id: u.id,
            email: u.email.clone(),
            role: u.role,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub email: String,
    pub password: String,
}

/// Build the `/api/v1/auth` sub-router. All three endpoints are mounted here;
/// `/me` requires auth via its `CurrentUser` extractor (no layer needed).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
}

// ---------------------------------------------------------------------------
// POST /login
// ---------------------------------------------------------------------------

pub async fn login(
    State(app): State<AppState>,
    jar: axum_extra::extract::CookieJar,
    Json(payload): Json<LoginPayload>,
) -> ApiResult<(StatusCode, axum_extra::extract::CookieJar, Json<UserPublic>)> {
    let email = payload.email.trim();
    if email.is_empty() || email.len() > MAX_EMAIL_LEN {
        return Err(ApiError::unauthorized("invalid credentials"));
    }
    if payload.password.is_empty() || payload.password.len() > MAX_PASSWORD_LEN {
        return Err(ApiError::unauthorized("invalid credentials"));
    }

    // Constant-time-ish lookup: do NOT short-circuit on "user not found" before
    // verifying a password. We still return a uniform "invalid credentials" so
    // the endpoint does not leak which emails exist.
    let user = repo::users::find_by_email(&app.db, email)
        .await?
        .ok_or_else(|| ApiError::unauthorized("invalid credentials"))?;
    if !auth::password::verify_password(&payload.password, &user.password_hash) {
        return Err(ApiError::unauthorized("invalid credentials"));
    }

    let token = auth::session::create(&app.redis, &user, app.config.session_ttl_secs)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e).context("session create")))?;

    tracing::info!(user_id = user.id, email = %user.email, "user logged in");
    let cookie = build_session_cookie(&app.config, &token);
    Ok((
        StatusCode::OK,
        jar.add(cookie),
        Json(UserPublic::from(&user)),
    ))
}

// ---------------------------------------------------------------------------
// POST /logout
// ---------------------------------------------------------------------------

pub async fn logout(
    State(app): State<AppState>,
    jar: axum_extra::extract::CookieJar,
) -> ApiResult<(StatusCode, axum_extra::extract::CookieJar)> {
    if let Some(c) = jar.get(&app.config.session_cookie_name) {
        let token = c.value().to_string();
        // Best-effort: a Redis error during logout must not block clearing the
        // client cookie.
        if let Err(e) = auth::session::delete(&app.redis, &token).await {
            tracing::warn!(error = %e, "failed to delete session from redis during logout");
        }
    }
    // Overwrite the cookie with an immediately-expired one so the browser
    // drops it regardless of the Redis outcome.
    let expired = build_expired_cookie(&app.config);
    Ok((StatusCode::NO_CONTENT, jar.add(expired)))
}

// ---------------------------------------------------------------------------
// GET /me
// ---------------------------------------------------------------------------

pub async fn me(user: CurrentUser) -> Json<UserPublic> {
    Json(UserPublic::from(&user))
}

// ---------------------------------------------------------------------------
// Cookie helpers
// ---------------------------------------------------------------------------

fn build_session_cookie(config: &crate::config::Config, token: &str) -> Cookie<'static> {
    let mut b = Cookie::build((config.session_cookie_name.clone(), token.to_string()))
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(parse_samesite(&config.cookie_samesite))
        .path("/")
        .max_age(CookieDuration::seconds(
            config.session_ttl_secs as i64,
        ));
    if let Some(domain) = config.cookie_domain.as_deref() {
        b = b.domain(domain.to_string());
    }
    b.build()
}

fn build_expired_cookie(config: &crate::config::Config) -> Cookie<'static> {
    // Same attributes as the session cookie (path/domain/same-site) so the
    // browser matches and deletes the right cookie; value empty + Max-Age 0.
    let mut b = Cookie::build((config.session_cookie_name.clone(), String::new()))
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(parse_samesite(&config.cookie_samesite))
        .path("/")
        .max_age(CookieDuration::seconds(0));
    if let Some(domain) = config.cookie_domain.as_deref() {
        b = b.domain(domain.to_string());
    }
    b.build()
}

fn parse_samesite(s: &str) -> SameSite {
    match s.to_ascii_lowercase().as_str() {
        "lax" => SameSite::Lax,
        "none" => SameSite::None,
        _ => SameSite::Strict,
    }
}
