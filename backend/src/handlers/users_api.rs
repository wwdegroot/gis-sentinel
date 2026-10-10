//! Admin-only user management API (task 2.7, users-admin-plan §2).
//!
//! Routes (mounted under `/api/v1/users`; GET/POST only — state changes are
//! POST sub-actions, same convention as `/alert-points`):
//!
//! | Method | Path             | Purpose |
//! |--------|------------------|---------|
//! | GET    | `/`              | list users (admin only; viewers 403 — account data) |
//! | POST   | `/`              | create user → 201 `UserAccount` |
//! | POST   | `/{id}/update`   | partial edit: `{email?, password?, role?}` → 200 |
//! | POST   | `/{id}/delete`   | delete → 204 |
//!
//! Access control mirrors `alert_points_api.rs`: handler-level `CurrentUser`
//! extractor (401 when absent) + local `require_admin` (403 for viewers).
//!
//! Business rules (plan Q3/Q5):
//! - cannot create/demote into a state with zero admins (last-admin guard),
//! - cannot delete yourself,
//! - `password` on edit is optional; absent = unchanged (argon2 rehash when set),
//! - **any** successful update/delete revokes that user's live sessions —
//!   role/email are baked into the session payload, so a stale session must
//!   not survive an edit (a demoted admin would keep admin rights until TTL).
//!
//! Error mapping: duplicate email → 409 `Conflict`; bad email / short password
//! → 422 `Validation`; unknown id → 404; rule violations (last admin, self
//! delete) → 409 `Conflict`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::{self, CurrentUser};
use crate::db::models::{Role, User};
use crate::db::repo;
use crate::handlers::error::{ApiError, ApiResult};
use crate::AppState;

/// Max email length accepted by the user endpoints (DoS guard, mirrors login).
const MAX_EMAIL_LEN: usize = 254; // RFC 5321
/// Min password length (plan §2: "min 8").
const MIN_PASSWORD_LEN: usize = 8;
/// Max password length (login DoS-guard style).
const MAX_PASSWORD_LEN: usize = 1024;

/// Public account row as returned by the user-management endpoints. Same
/// shape as [`UserPublic`] plus timestamps for the admin table.
#[derive(Debug, serde::Serialize)]
pub struct UserAccount {
    pub id: i64,
    pub email: String,
    pub role: Role,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<&User> for UserAccount {
    fn from(u: &User) -> Self {
        Self {
            id: u.id,
            email: u.email.clone(),
            role: u.role,
            created_at: u.created_at,
            updated_at: u.updated_at,
        }
    }
}

/// Build the `/api/v1/users` sub-router. Every endpoint is admin-only.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_users).post(create_user))
        .route("/{id}/update", post(update_user))
        .route("/{id}/delete", post(delete_user))
}

// ---------------------------------------------------------------------------
// Validation (pure, unit-tested below)
// ---------------------------------------------------------------------------

/// Validate an email for create/update: trimmed non-empty, single `@`, RFC
/// length bound. Deliberately lenient on local/domain shape — real validation
/// happens at login (a wrong address simply never receives anything).
pub(crate) fn validate_email(email: &str) -> Result<(), &'static str> {
    if email.is_empty() {
        return Err("email must not be empty");
    }
    if email.len() > MAX_EMAIL_LEN {
        return Err("email is too long");
    }
    if email.chars().any(char::is_whitespace) {
        return Err("email must not contain whitespace");
    }
    let at_count = email.matches('@').count();
    if at_count != 1 {
        return Err("email must contain exactly one '@'");
    }
    let Some((_, domain)) = email.split_once('@') else {
        // unreachable after the count check above, but avoid `expect`
        return Err("email must contain exactly one '@'");
    };
    if domain.is_empty() || !domain.contains('.') {
        return Err("email domain must look like a domain (e.g. example.com)");
    }
    Ok(())
}

/// Validate a password for create/rotation: length within [8, MAX].
pub(crate) fn validate_password(password: &str) -> Result<(), &'static str> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err("password must be at least 8 characters");
    }
    if password.len() > MAX_PASSWORD_LEN {
        return Err("password is too long");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

async fn list_users(
    State(app): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<UserAccount>>> {
    require_admin(&user)?;

    let users = repo::users::list(&app.db).await?;
    Ok(Json(users.iter().map(UserAccount::from).collect()))
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct NewUserPayload {
    pub email: String,
    pub password: String,
    pub role: Role,
}

async fn create_user(
    State(app): State<AppState>,
    user: CurrentUser,
    Json(payload): Json<NewUserPayload>,
) -> ApiResult<(StatusCode, Json<UserAccount>)> {
    require_admin(&user)?;
    validate_user_fields(Some(&payload.email), Some(&payload.password))?;

    let password_hash = auth::password::hash_password(&payload.password)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e).context("password hash")))?;

    // Duplicate email → 409 (unique violation), everything else passthrough.
    let created = repo::users::create(&app.db, payload.email.trim(), &password_hash, payload.role)
        .await
        .map_err(map_user_db_err)?;

    tracing::info!(user_id = created.id, "user created by admin");
    Ok((StatusCode::CREATED, Json(UserAccount::from(&created))))
}

// ---------------------------------------------------------------------------
// Update (partial patch)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct UserPatchPayload {
    pub email: Option<String>,
    /// When present, the password is rotated (argon2 rehash). Absent = unchanged.
    pub password: Option<String>,
    pub role: Option<Role>,
}

async fn update_user(
    State(app): State<AppState>,
    caller: CurrentUser,
    Path(id): Path<i64>,
    Json(payload): Json<UserPatchPayload>,
) -> ApiResult<Json<UserAccount>> {
    require_admin(&caller)?;

    if payload.email.is_none() && payload.password.is_none() && payload.role.is_none() {
        return Err(ApiError::BadRequest(
            "update payload must set at least one of email, password, role".to_string(),
        ));
    }

    let target = repo::users::find_by_id(&app.db, id)
        .await?
        .ok_or(ApiError::NotFound("user"))?;

    if let Some(email) = &payload.email {
        validate_user_fields(Some(email), None)?;
    }
    if let Some(password) = &payload.password {
        validate_user_fields(None, Some(password))?;
    }

    // Last-admin guard: demoting (or deleting) the final admin would lock
    // every admin surface out. Only fires when the target is an admin and the
    // change removes that status.
    let demotes_last_admin = target.role == Role::Admin
        && payload.role == Some(Role::Viewer)
        && repo::users::count_admins(&app.db).await? <= 1;
    if demotes_last_admin {
        return Err(ApiError::Conflict(
            "cannot demote the last admin".to_string(),
        ));
    }

    // Self-guard: an admin may edit their own email/password, but demoting
    // themselves would immediately revoke their own session (the role is
    // baked into the session payload) — reject it outright; another admin can
    // perform the change instead. Self-delete is rejected in `delete_user`.
    if caller.id == id && payload.role == Some(Role::Viewer) && target.role == Role::Admin {
        return Err(ApiError::Conflict(
            "cannot change your own role; ask another admin".to_string(),
        ));
    }

    let password_hash = match &payload.password {
        Some(pw) => Some(
            auth::password::hash_password(pw)
                .map_err(|e| ApiError::Internal(anyhow::anyhow!(e).context("password hash")))?,
        ),
        None => None,
    };

    let mut updated = target;
    if let Some(email) = &payload.email {
        updated = repo::users::update_email(&app.db, id, email.trim())
            .await?
            .ok_or(ApiError::NotFound("user"))?;
    }
    if let Some(hash) = &password_hash {
        updated = repo::users::update_password(&app.db, id, hash)
            .await?
            .ok_or(ApiError::NotFound("user"))?;
    }
    if let Some(role) = payload.role {
        updated = repo::users::update_role(&app.db, id, role)
            .await?
            .ok_or(ApiError::NotFound("user"))?;
    }

    // Role/email/password are baked into the session payload → revoke all
    // live sessions of the edited user (plan §3). For self-edits this kills
    // the caller's own session: their next request 401s → redirect to /login.
    revoke_sessions_quietly(&app, updated.id).await;

    tracing::info!(user_id = updated.id, "user updated by admin");
    Ok(Json(UserAccount::from(&updated)))
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

async fn delete_user(
    State(app): State<AppState>,
    caller: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_admin(&caller)?;

    if caller.id == id {
        return Err(ApiError::Conflict("cannot delete yourself".to_string()));
    }

    let target = repo::users::find_by_id(&app.db, id)
        .await?
        .ok_or(ApiError::NotFound("user"))?;

    // Last-admin guard (Q3): deleting the final admin locks every admin
    // surface out.
    if target.role == Role::Admin && repo::users::count_admins(&app.db).await? <= 1 {
        return Err(ApiError::Conflict(
            "cannot delete the last admin".to_string(),
        ));
    }

    let deleted = repo::users::delete(&app.db, id).await?;
    if !deleted {
        return Err(ApiError::NotFound("user"));
    }

    // The user is gone; nothing may keep a session alive.
    revoke_sessions_quietly(&app, id).await;

    tracing::info!(user_id = id, "user deleted by admin");
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Reject non-admin users with 403 (mirrors `alert_points_api::require_admin`).
fn require_admin(user: &CurrentUser) -> Result<(), ApiError> {
    if user.role.at_least(Role::Admin) {
        Ok(())
    } else {
        Err(ApiError::forbidden("admin role required for this action"))
    }
}

/// Shared field validation for create/update; returns a 422 `Validation`
/// error with one entry per offending field. `None` = field absent (skip).
fn validate_user_fields(email: Option<&str>, password: Option<&String>) -> Result<(), ApiError> {
    let mut fields = Vec::new();
    if let Some(email) = email {
        if let Err(msg) = validate_email(email.trim()) {
            fields.push(("email".to_string(), msg.to_string()));
        }
    }
    if let Some(pw) = password {
        if let Err(msg) = validate_password(pw) {
            fields.push(("password".to_string(), msg.to_string()));
        }
    }
    if fields.is_empty() {
        Ok(())
    } else {
        Err(ApiError::Validation(fields))
    }
}

/// Map repo errors from user writes: the unique constraint on `users.email`
/// becomes 409 `Conflict` (a clearer message than the generic constraint
/// mapping in `ApiError::from(sqlx::Error)`); everything else passthrough.
fn map_user_db_err(e: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db_err) = &e {
        if db_err.kind() == sqlx::error::ErrorKind::UniqueViolation {
            tracing::warn!(
                constraint = db_err.constraint().unwrap_or("?"),
                "duplicate email"
            );
            return ApiError::Conflict("a user with this email already exists".to_string());
        }
    }
    ApiError::from(e)
}

/// Revoke the user's live sessions; Redis failures are logged, not surfaced
/// (the DB write already succeeded — mirroring the logout best-effort path).
async fn revoke_sessions_quietly(app: &AppState, user_id: i64) {
    if let Err(e) = auth::session::revoke_user_sessions(&app.redis, user_id).await {
        tracing::warn!(user_id, error = %e, "failed to revoke user sessions after update");
    }
}

// ---------------------------------------------------------------------------
// Tests: pure validation + guard logic (no Redis / DB)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_validation_accepts_normal_addresses() {
        assert!(validate_email("admin@example.com").is_ok());
        assert!(validate_email("a.b+c@sub.example.co.uk").is_ok());
    }

    #[test]
    fn email_validation_rejects_bad_input() {
        assert!(validate_email("").is_err());
        assert!(validate_email("no-at-sign").is_err());
        assert!(validate_email("a@b@c").is_err());
        assert!(validate_email("user@").is_err());
        assert!(validate_email("user@nodot").is_err());
        assert!(validate_email("two words@example.com").is_err());
        assert!(validate_email(&"a".repeat(MAX_EMAIL_LEN + 1)).is_err());
    }

    #[test]
    fn password_validation_bounds() {
        assert!(validate_password(&"p".repeat(MIN_PASSWORD_LEN)).is_ok());
        assert!(validate_password(&"p".repeat(MIN_PASSWORD_LEN - 1)).is_err());
        assert!(validate_password("").is_err());
        assert!(validate_password(&"p".repeat(MAX_PASSWORD_LEN + 1)).is_err());
    }

    #[test]
    fn field_errors_carry_field_names() {
        let err = validate_user_fields(Some("bad"), Some(&"short".to_string())).unwrap_err();
        match err {
            ApiError::Validation(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0, "email");
                assert_eq!(fields[1].0, "password");
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn field_validation_skips_absent_fields() {
        // update path: only email provided
        assert!(validate_user_fields(Some("ok@example.com"), None).is_ok());
        // empty string counts as present → rejected (not silently skipped)
        assert!(validate_user_fields(Some(""), None).is_err());
    }

    #[test]
    fn require_admin_rejects_viewers() {
        let admin = test_user(Role::Admin);
        let viewer = test_user(Role::Viewer);
        assert!(require_admin(&admin).is_ok());
        assert!(require_admin(&viewer).is_err());
    }

    /// Minimal `CurrentUser` for guard tests (source is irrelevant here).
    fn test_user(role: Role) -> CurrentUser {
        CurrentUser {
            id: 1,
            email: "admin@example.com".to_string(),
            role,
            source: auth::AuthSource::Session,
        }
    }
}
