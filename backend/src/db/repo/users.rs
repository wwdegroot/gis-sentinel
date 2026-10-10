//! User repository (auth, Option B).
//!
//! Backs `/api/v1/auth/{login,logout,me}`, the admin user-management
//! endpoints (`/api/v1/users`, task 2.7), and the startup bootstrap-admin
//! upsert. Server-side sessions live in Redis (`crate::auth::session`); this
//! module only touches the `users` table (identity + argon2 hashes).

use crate::db::models::{Role, User};
use sqlx::PgPool;

// ---------------------------------------------------------------------------
// User management (task 2.7, admin-only)
// ---------------------------------------------------------------------------

/// List all users ordered by creation (admin user-management table).
pub async fn list(pool: &PgPool) -> Result<Vec<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"
        SELECT id, email, password_hash, role AS "role: Role", created_at, updated_at
        FROM users
        ORDER BY created_at, id
        "#
    )
    .fetch_all(pool)
    .await
}

/// Count users with the admin role (last-admin guard).
pub async fn count_admins(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let count: i64 = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "count!" FROM users WHERE role = 'admin'
        "#
    )
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Change a user's email. Callers must revoke the user's sessions afterwards
/// (the email is baked into the session payload).
pub async fn update_email(
    pool: &PgPool,
    id: i64,
    email: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"
        UPDATE users SET email = $2
        WHERE id = $1
        RETURNING id, email, password_hash, role AS "role: Role", created_at, updated_at
        "#,
        id,
        email,
    )
    .fetch_optional(pool)
    .await
}

/// Change a user's password (`password_hash` must already be argon2-hashed).
/// Callers must revoke the user's sessions afterwards.
pub async fn update_password(
    pool: &PgPool,
    id: i64,
    password_hash: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"
        UPDATE users SET password_hash = $2
        WHERE id = $1
        RETURNING id, email, password_hash, role AS "role: Role", created_at, updated_at
        "#,
        id,
        password_hash,
    )
    .fetch_optional(pool)
    .await
}

/// Change a user's role. Callers must revoke the user's sessions afterwards
/// (the role is baked into the session payload).
pub async fn update_role(pool: &PgPool, id: i64, role: Role) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"
        UPDATE users SET role = $2
        WHERE id = $1
        RETURNING id, email, password_hash, role AS "role: Role", created_at, updated_at
        "#,
        id,
        role as Role,
    )
    .fetch_optional(pool)
    .await
}

/// Delete a user by id. Returns `false` when the id did not exist.
/// (`alert_points` rows have no user ownership, so nothing cascades.)
pub async fn delete(pool: &PgPool, id: i64) -> Result<bool, sqlx::Error> {
    let result: sqlx::postgres::PgQueryResult = sqlx::query!("DELETE FROM users WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Auth lookups
// ---------------------------------------------------------------------------

/// Look up a user by email (login flow).
pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"
        SELECT id, email, password_hash, role AS "role: Role", created_at, updated_at
        FROM users
        WHERE email = $1
        "#,
        email,
    )
    .fetch_optional(pool)
    .await
}

/// Look up a user by id (`/auth/me`, session deserialization).
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"
        SELECT id, email, password_hash, role AS "role: Role", created_at, updated_at
        FROM users
        WHERE id = $1
        "#,
        id,
    )
    .fetch_optional(pool)
    .await
}

/// Create a new user with the given role.
pub async fn create(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
    role: Role,
) -> Result<User, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"
        INSERT INTO users (email, password_hash, role)
        VALUES ($1, $2, $3)
        RETURNING id, email, password_hash, role AS "role: Role", created_at, updated_at
        "#,
        email,
        password_hash,
        role as Role,
    )
    .fetch_one(pool)
    .await
}

/// Insert or update the bootstrap admin (idempotent startup hook, Q5).
///
/// On each boot where `BOOTSTRAP_ADMIN_EMAIL`/`BOOTSTRAP_ADMIN_PASSWORD` are set,
/// the admin row is created if missing, or its password hash + role are refreshed
/// to the env value. Returns the resulting user.
pub async fn upsert_bootstrap_admin(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
) -> Result<User, sqlx::Error> {
    // ON CONFLICT (email) DO UPDATE refreshes the password (so rotating the env
    // value rotates the admin) and pins role to 'admin'.
    sqlx::query_as!(
        User,
        r#"
        INSERT INTO users (email, password_hash, role)
        VALUES ($1, $2, 'admin')
        ON CONFLICT (email) DO UPDATE
            SET password_hash = EXCLUDED.password_hash,
                role = 'admin'
        RETURNING id, email, password_hash, role AS "role: Role", created_at, updated_at
        "#,
        email,
        password_hash,
    )
    .fetch_one(pool)
    .await
}
