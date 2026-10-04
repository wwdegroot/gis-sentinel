-- Authentication (Option B): local user accounts for session-cookie auth.
--
-- A `users` table backs the `/api/v1/auth/{login,logout,me}` endpoints. Server-side
-- sessions live in Redis (key `sentinel:session:{token}`); this table only stores
-- identity + argon2 password hashes. The `role` column drives `require_role` guards
-- (viewer = read-only on GET routes; admin = full access incl. POST sub-actions).
--
-- The reverse-proxy/SSO path (Option D) reuses this table: an `X-Authenticated-User`
-- header from a trusted proxy upserts/looks up a row by email, so handlers and role
-- guards stay unchanged when D is enabled later.

CREATE TABLE users (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'admin' CHECK (role IN ('admin', 'viewer')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_users_email ON users (email);

-- Keep users.updated_at current automatically (mirrors the alert_points trigger).
CREATE TRIGGER trg_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
