//! User repository (auth, Option B).
//!
//! Backs `/api/v1/auth/{login,logout,me}` and the startup bootstrap-admin upsert.
//! Server-side sessions live in Redis (`crate::auth::session`); this module only
//! touches the `users` table (identity + argon2 hashes).

use crate::db::models::{Role, User};
use sqlx::PgPool;

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
