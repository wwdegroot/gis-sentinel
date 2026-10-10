//! Redis-backed server-side sessions (auth, Option B, Q1 = Redis).
//!
//! A session is a random UUID v4 token stored in a `HttpOnly` cookie; the token
//! maps to a Redis key `sentinel:session:{token}` whose value is the minimal
//! user identity needed to authorize a request without a DB hit. TTL is sliding
//! (refreshed on each authenticated request); logout = `DEL`.

use crate::db::models::{Role, User};
use crate::queue::{QueueError, RedisPool};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Redis key prefix for session tokens.
pub const SESSION_KEY_PREFIX: &str = "sentinel:session:";

/// Redis key prefix for the per-user session-token index (task 2.7). Each set
/// holds every live token of one user so role/email/password edits or a delete
/// can revoke all of that user's sessions at once. The set carries the same
/// TTL as the session it indexes, so it self-expires with its last session.
pub const SESSION_INDEX_KEY_PREFIX: &str = "sentinel:session_idx:";

/// The identity payload stored per session token (JSON).
///
/// Mirrors the fields `require_auth` needs to build a `CurrentUser` without a
/// DB lookup on every request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub user_id: i64,
    pub email: String,
    pub role: Role,
}

impl From<&User> for SessionData {
    fn from(u: &User) -> Self {
        Self {
            user_id: u.id,
            email: u.email.clone(),
            role: u.role,
        }
    }
}

/// Create a new session: generate a token, store `data` with `ttl_secs`, return
/// the token (to be set as a cookie by the caller).
pub async fn create(pool: &RedisPool, user: &User, ttl_secs: u64) -> Result<String, QueueError> {
    let token = Uuid::new_v4().to_string();
    let payload = serde_json::to_string(&SessionData::from(user))?;
    let key = format!("{SESSION_KEY_PREFIX}{token}");
    let mut conn = pool.get().await?;
    let mut pipe = redis::pipe();
    pipe.set_ex(key, payload, ttl_secs).ignore();
    // Index set for `revoke_user_sessions`: TTL'd like the session so an
    // abandoned index self-expires with its last session.
    pipe.sadd(format!("{SESSION_INDEX_KEY_PREFIX}{}", user.id), &token)
        .ignore();
    pipe.expire(
        format!("{SESSION_INDEX_KEY_PREFIX}{}", user.id),
        ttl_secs as i64,
    )
    .ignore();
    pipe.query_async::<()>(&mut conn).await?;
    Ok(token)
}

/// Look up a session by token and refresh its TTL (sliding expiry).
/// Returns `None` if the token is unknown/expired.
pub async fn get(
    pool: &RedisPool,
    token: &str,
    ttl_secs: u64,
) -> Result<Option<SessionData>, QueueError> {
    let key = format!("{SESSION_KEY_PREFIX}{token}");
    let mut conn = pool.get().await?;
    let payload: Option<String> = conn.get(&key).await?;
    match payload {
        Some(p) => {
            // Sliding TTL: refresh on read. A race between GET and EXPIRE (the
            // key expires in between) only costs a spurious EXPIRE on a missing
            // key, which is a harmless no-op.
            let _: bool = conn.expire(&key, ttl_secs as i64).await?;
            let data: SessionData = serde_json::from_str(&p)?;
            Ok(Some(data))
        }
        None => Ok(None),
    }
}

/// Delete a session (logout). Best-effort: a missing key is not an error.
pub async fn delete(pool: &RedisPool, token: &str) -> Result<(), QueueError> {
    let key = format!("{SESSION_KEY_PREFIX}{token}");
    let mut conn = pool.get().await?;
    conn.del::<_, ()>(key).await?;
    Ok(())
}

/// Revoke every live session of a user (task 2.7): read the token index set,
/// `DEL` each session key, then drop the index set itself.
///
/// Called after any successful user update/delete — role, email and password
/// are baked into the session payload, so a stale session must not survive an
/// edit (e.g. a demoted admin would otherwise keep admin rights until TTL).
pub async fn revoke_user_sessions(pool: &RedisPool, user_id: i64) -> Result<(), QueueError> {
    let index_key = format!("{SESSION_INDEX_KEY_PREFIX}{user_id}");
    let mut conn = pool.get().await?;
    let tokens: Vec<String> = conn.smembers(&index_key).await?;
    if !tokens.is_empty() {
        let keys: Vec<String> = tokens
            .iter()
            .map(|t| format!("{SESSION_KEY_PREFIX}{t}"))
            .collect();
        conn.del::<_, ()>(keys).await?;
    }
    conn.del::<_, ()>(index_key).await?;
    Ok(())
}

/// Revoke every live session of a user **except** one token (self-service
/// password change): the current device stays logged in, other devices are
/// signed out. The kept token stays in the index set.
pub async fn revoke_other_user_sessions(
    pool: &RedisPool,
    user_id: i64,
    keep_token: &str,
) -> Result<(), QueueError> {
    let index_key = format!("{SESSION_INDEX_KEY_PREFIX}{user_id}");
    let mut conn = pool.get().await?;
    let tokens: Vec<String> = conn.smembers(&index_key).await?;
    let stale: Vec<String> = tokens.into_iter().filter(|t| t != keep_token).collect();
    if !stale.is_empty() {
        let keys: Vec<String> = stale
            .iter()
            .map(|t| format!("{SESSION_KEY_PREFIX}{t}"))
            .collect();
        conn.del::<_, ()>(keys).await?;
        // SREM each stale token from the index set.
        for t in &stale {
            let _: usize = conn
                .srem(format!("{SESSION_INDEX_KEY_PREFIX}{user_id}"), t)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{Role, User};
    use chrono::Utc;

    fn test_user(id: i64) -> User {
        User {
            id,
            email: "roundtrip@example.com".to_string(),
            password_hash: "argon2$hash".to_string(),
            role: Role::Viewer,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn pool() -> Result<RedisPool, Box<dyn std::error::Error>> {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        Ok(crate::queue::connect(&url)?)
    }

    /// Round-trip against a live Redis/Valkey (users-admin-plan §6):
    /// `create` indexes the token → `revoke_user_sessions` kills it →
    /// `get` returns `None`. Run explicitly:
    /// `cargo test --features smoke-tests -- --ignored session_revocation_round_trip`
    #[tokio::test]
    #[ignore = "requires REDIS_URL pointing at a live redis/valkey"]
    async fn session_revocation_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let pool = pool()?;
        let user = test_user(424_242);

        // two sessions for the same user
        let t1 = create(&pool, &user, 120).await?;
        let t2 = create(&pool, &user, 120).await?;
        assert!(get(&pool, &t1, 120).await?.is_some());
        assert!(get(&pool, &t2, 120).await?.is_some());

        // revoke all → both tokens die, index set gone
        revoke_user_sessions(&pool, user.id).await?;
        assert!(get(&pool, &t1, 120).await?.is_none());
        assert!(get(&pool, &t2, 120).await?.is_none());
        let members: Vec<String> = pool
            .get()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?
            .smembers(format!("{SESSION_INDEX_KEY_PREFIX}{}", user.id))
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        assert!(members.is_empty());
        Ok(())
    }

    /// `revoke_other_user_sessions` keeps the current device signed in while
    /// every other session of the user dies (self-service password change).
    #[tokio::test]
    #[ignore = "requires REDIS_URL pointing at a live redis/valkey"]
    async fn revoke_other_sessions_keeps_current() -> Result<(), Box<dyn std::error::Error>> {
        let pool = pool()?;
        let user = test_user(424_243);

        let keep = create(&pool, &user, 120).await?;
        let other = create(&pool, &user, 120).await?;

        revoke_other_user_sessions(&pool, user.id, &keep).await?;
        assert!(get(&pool, &keep, 120).await?.is_some());
        assert!(get(&pool, &other, 120).await?.is_none());

        // cleanup
        revoke_user_sessions(&pool, user.id).await?;
        Ok(())
    }
}
