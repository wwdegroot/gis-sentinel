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
    conn.set_ex::<_, _, ()>(key, payload, ttl_secs).await?;
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
