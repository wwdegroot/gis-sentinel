# Auth Plan — Session Cookies (B) now, Reverse-Proxy SSO (D) later

**Goal:** close the currently-open REST + WebSocket surface with real per-user auth
(Option B), implemented so that delegating to an external IdP via a trusted reverse
proxy (Option D) becomes a config flip — no rewrite.

**Decisions confirmed with user:** start with B for a small internal team; HTTPS in
prod; D is the documented upgrade path (existing SSO/IdP later).

---

## 0. Design principle: one `CurrentUser`, swappable source

All protected routes depend on a single extractor that yields a `CurrentUser`. The
*source* of that user is decided by config:

| Mode | Source | When |
|---|---|---|
| `SESSION` (default, B) | Redis session looked up from `sentinel_session` cookie | local/team logins |
| `TRUSTED_PROXY` (D, later) | `X-Authenticated-User` header, accepted only when `remote_ip == TRUSTED_PROXY_IP` | behind Caddy/Authentik/oauth-proxy |

Both can be enabled at once (proxy auth for SSO users, session auth for the local
admin). This is the whole of the "B → D" upgrade story: the `users` table, role
guards, and every handler stay untouched.

---

## 1. Decisions (recommended; ❓ = needs your confirmation)

| # | Question | Options | Recommendation |
|---|---|---|---|
| Q1 | Session store | a) **Redis** (TTL auto-expiry, no DB hit, reuse existing pool), b) Postgres `sessions` table | **a** — Redis already in `AppState`; instant revocation = `DEL` |
| Q2 | Password hashing | a) `argon2`, b) `bcrypt` | **a** — modern default (OWASP) |
| Q3 | Role model now | a) `admin` only, b) **`admin` + `viewer`** (viewer read-only; POST routes require admin) | **b** — confirmed; `require_role(Role::Admin)` on POST routes, frontend hides admin buttons by role |
| Q4 | CSRF posture | a) **`SameSite=Strict` cookie + JSON-only `Content-Type` enforcement** (no token), b) add a double-submit `X-CSRF-Token` | **a** — cross-site forms can't forge a JSON request; revisit if we add form endpoints |
| Q5 | First-admin bootstrap | a) **env-var upsert on startup** (`BOOTSTRAP_ADMIN_EMAIL`/`_PASSWORD`), b) `--init-admin` CLI subcommand, c) both | **a** — fits the `envy` config story, idempotent, docker-compose-friendly |
| Q6 | Cookie value | random UUID v4 session token (122 bits) stored in Redis, cookie is *not* signed (tampering → Redis miss → 401) | **a** — no signing crate needed |
| Q7 | WebSocket auth | session cookie rides the WS upgrade automatically (same-origin); `?token=` fallback **not** needed under B | cookie-only |

✅ **All confirmed**: Q1 = Redis, Q3 = admin+viewer now, Q4 = SameSite=Strict + JSON-only. Q2/Q5/Q6/Q7 as recommended.

---

## 2. Dependencies (`backend/Cargo.toml`)

| Crate | Version | Why |
|---|---|---|
| `argon2` | `0.5` | password hash/verify |
| `axum-extra` | `0.12` (+feature `cookie`) | `CookieJar` extractor for reading/setting the session cookie; already depended on for `typed-header` |
| `uuid` | `1` (+feature `v4`) | session-token generation; `v7`+`serde` already on |

No `jsonwebtoken`, no `rand`, no `tower-cookies`. After adding any `query!` macros:
`cargo sqlx prepare` (keep `SQLX_OFFLINE=true` build green).

---

## 3. DB migration `0004_users.sql`

```sql
CREATE TABLE users (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'admin' CHECK (role IN ('admin','viewer')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_users_email ON users (email);
-- BEFORE UPDATE trigger keeping updated_at current (mirrors alert_points pattern).
```

Sessions live in Redis (Q1-a): key `sentinel:session:{token}` → JSON `{user_id, role, email}`,
TTL = `SESSION_TTL_SECONDS` (sliding: refreshed on each authenticated request).

---

## 4. Module layout

```
backend/src/auth/
├── mod.rs          # CurrentUser, AuthSource, require_auth middleware + require_role guard
├── password.rs     # argon2 hash_password / verify_password
├── session.rs      # Redis store: create(token,user) / get / touch / delete
└── users.rs        # repo: find_by_email, find_by_id, upsert_bootstrap_admin
backend/src/handlers/
├── auth_api.rs     # NEW: POST /auth/login, /auth/logout, GET /auth/me
└── ...
```

`auth::users` is a repo module (lives in the lib crate like `db::repo`) so the `.sqlx`
offline cache covers it.

---

## 5. Routes (public vs protected)

```rust
// public
.route("/api/v1/auth/login", post(login))
.route("/api/v1/auth/logout", post(logout))
// /healthz, /livez, and the SPA fallback (static_handler) stay public —
// the bundle has no secrets; the frontend calls /auth/me and redirects to /login on 401.

// protected (require_auth layer applied to the nested router)
.nest("/api/v1/alert-points", alert_points_api::router().layer(require_auth))
.route("/api/v1/auth/me", get(me).layer(require_auth))
.route("/ws/sentinel", get(ws_sentinel_handler).layer(require_auth))
```

`require_auth` = `from_fn` middleware resolving `CurrentUser` via the enabled source(s);
rejects with `401` JSON `ApiError` otherwise. `require_role(r)` builds on it: `403` if the
resolved user's role is below `r`. Handlers take `Extension(CurrentUser)`.

**Role split** (inside `alert_points_api::router()`): read routes (`GET /`, `GET /{id}`,
`GET /{id}/{history,incidents,history/export}`) → `require_auth` (admin or viewer);
write routes (`POST /`, `POST /{id}/{update,delete,test}`) → `require_role(Role::Admin)`.

---

## 6. `CurrentUser` + source resolution (B → D in one place)

```rust
pub struct CurrentUser { id, email, role, source: AuthSource }
pub enum AuthSource { Session, TrustedProxy }

// require_auth resolution order:
// 1. if ENABLE_TRUSTED_PROXY_AUTH && remote_ip == TRUSTED_PROXY_IP
//      && X-Authenticated-User present → upsert/lookup user by email → TrustedProxy
// 2. else read `sentinel_session` cookie → Redis GET → Session
// 3. else 401
```

Enabling D later = set `ENABLE_TRUSTED_PROXY_AUTH=true` + `TRUSTED_PROXY_IP=…` + run
the proxy. The `users` row for an SSO user is upserted on first proxied request (or
pre-provisioned). No handler changes.

---

## 7. Config additions (`backend/src/config.rs`, `.env.example`)

| Env var | Default | Purpose |
|---|---|---|
| `SESSION_COOKIE_NAME` | `sentinel_session` | cookie name |
| `SESSION_TTL_SECONDS` | `604800` (7 d) | Redis session TTL, sliding |
| `COOKIE_SECURE` | `true` | `Secure` flag (true in prod over HTTPS) |
| `COOKIE_SAMESITE` | `strict` | `SameSite` (`strict`/`lax`/`none`) |
| `COOKIE_DOMAIN` | _unset_ | optional cookie domain |
| `BOOTSTRAP_ADMIN_EMAIL` | _unset_ | startup upsert of first admin (Q5) |
| `BOOTSTRAP_ADMIN_PASSWORD` | _unset_ | paired with above |
| `ENABLE_TRUSTED_PROXY_AUTH` | `false` | D upgrade switch |
| `TRUSTED_PROXY_IP` | _unset_ | only accept header from this IP |

---

## 8. Admin bootstrap (Q5-a)

On startup, after migrations: if both `BOOTSTRAP_ADMIN_EMAIL` and
`BOOTSTRAP_ADMIN_PASSWORD` are set, `users::upsert_bootstrap_admin(email, password)` —
hashes with argon2 and inserts-or-updates the row (idempotent; password rotated to the
env value on each boot when set). Logged at `info!`. If unset, no admin exists until
one is created (future admin CLI / D provisioning).

---

## 9. Frontend changes

| File | Change |
|---|---|
| `src/lib/api.ts` | add `login`, `logout`, `me`; credentials: same-origin (default); on `401` throw `UnauthorizedError` |
| `src/lib/auth.svelte.ts` | NEW — `currentUser` `$state`, `init()` calls `me()` on app load, `login`/`logout` actions; `isAdmin` derived for button gating |
| `src/routes/login/+page.svelte` | NEW — email/password form, POST `/auth/login`, redirect to `/` on success |
| `src/routes/+layout.svelte` | `onMount` → `auth.init()`; if 401 → `goto('/login')`; nav shows email + Logout button |
| `src/lib/sentinelSocket.svelte.ts` | **no change** — cookie rides the WS upgrade same-origin |
| `src/routes/services/+page.svelte` | hide Create/Edit/Delete/Test-now buttons when `!auth.isAdmin` (viewer sees read-only table) |
| `src/lib/types.ts` | add `User { id, email, role }`, `LoginPayload` |

SPA fallback stays public; the frontend decides gating via `/auth/me`.

---

## 10. CSRF posture (Q4-a)

- Session cookie: `HttpOnly`, `Secure` (prod), `SameSite=Strict`, `Path=/`.
- All mutation endpoints are JSON-only (`axum::Json<T>` rejects `application/x-www-form-urlencoded`
  with 415) → a cross-site form POST cannot forge a valid request.
- Net: classic CSRF vector is closed without a token. Documented as a decision; revisit
  only if we add form-encodable endpoints.

---

## 11. Tests (feeds task 5.1)

- `auth/password.rs` — hash/verify round trip; wrong-password fails; re-verify after hash.
- `auth/session.rs` — `#[ignore]` Redis round trip: create → get → touch (TTL refresh) → delete.
- `tests/api_auth.rs` — `login` 200 + Set-Cookie; `me` 401 without cookie, 200 with;
  protected alert-point route 401 without, 200 with; `logout` clears session (subsequent `me` 401).
- `tests/bootstrap_admin.rs` — `#[ignore]`: env-var upsert creates admin, idempotent on re-run.

---

## 12. Verification plan

1. `cargo build` (offline `.sqlx`), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
2. `cargo test` — unit + `#[ignore]` Redis/PG tests against `postgres-gs` / `valkey-gs`.
3. Manual against live stack:
   - boot with `BOOTSTRAP_ADMIN_*` → admin row exists;
   - `curl /api/v1/alert-points` → 401; `curl /auth/login` → 200 + cookie; repeat with cookie → 200;
   - browser: `/` redirects to `/login`, login succeeds, dashboard loads, WS snapshot arrives (cookie auth), logout works;
   - `auth/me` returns the user.
4. D dry-run (no proxy): with `ENABLE_TRUSTED_PROXY_AUTH=true` + a fake `X-Authenticated-User`
   from a non-trusted IP → 401 (proves the IP guard).

---

## 13. Order of work

| Step | Scope | Verify |
|---|---|---|
| 1 | migration `0004`, `auth/users` repo, `auth/password`, `auth/session` (Redis) | `cargo sqlx prepare`; `#[ignore]` round-trips |
| 2 | `CurrentUser` + `require_auth` middleware (+ `require_role` guard) | unit |
| 3 | `handlers/auth_api` (login/logout/me), bootstrap-admin startup hook | `tests/api_auth.rs` |
| 4 | wire `require_auth` onto alert-points nest + `/auth/me` + `/ws/sentinel`; config knobs | live `curl` + WS |
| 5 | frontend: `auth.svelte.ts`, `/login`, layout gate, nav logout, `api.ts` | svelte-check, browser flow |
| 6 | full pass | `cargo test`, `pnpm check && pnpm build`, smoke against `postgres-gs` |

---

## 14. Explicit non-goals (deferred)

- Option D (trusted-proxy/SSO) wiring — only the *hook* (`ENABLE_TRUSTED_PROXY_AUTH`)
  lands now; running an IdP + enabling it is a later task.
- `viewer` role read-only enforcement — **now in scope** (Q3-b): `require_role(Admin)` on POST routes; frontend hides admin-only buttons.
- Per-target ownership / multi-tenancy.
- Password reset / email verification flow (local admin only for now).
- MFA (comes free with D via the IdP).
