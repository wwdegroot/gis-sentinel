# User Admin Plan — manage users (admin-only CRUD UI + endpoints)

**Status:** ✅ **IMPLEMENTED & VERIFIED** (2026-10-10) — tasks 2.7 + 3.7 complete; implementation report in §10. Remaining: the fake-based `tests/api_users.rs` integration harness (§10.6).

**Goal:** let logged-in admins add/remove/edit users from a `/users` page.
Viewers keep read-only access everywhere (auth-plan Q3-b); the user list itself is
**admin-only** (viewers never see the page, the nav link, or the endpoints — 403).

**Grounded in:** auth-plan implemented state (`docs/implemented/auth-plan.md`) —
Option B sessions in Redis, `users` table (migration `0004_users.sql`),
`CurrentUser` extractor, argon2 passwords, `role` enum `admin|viewer`.
Like the auth plan: Option B now, the Option D trusted-proxy hook stays untouched.

---

## 0. Current state (verified in repo)

| Piece | Where | Status |
| --- | --- | --- |
| `users` table + `role` CHECK ('admin','viewer') | `migrations/0004_users.sql` | ✅ exists — **no new migration needed** |
| users repo | `backend/src/db/repo/users.rs` | `find_by_email`, `find_by_id`, `create`, `upsert_bootstrap_admin` — **missing `list`, `delete`, `update_*`, `count_admins`** |
| password hashing | `backend/src/auth/password.rs` | argon2 `hash_password`/`verify_password` — reuse as-is |
| session store | `backend/src/auth/session.rs` | Redis `create`/`get` (sliding TTL)/`delete` — reuse as-is |
| `CurrentUser` extractor | `backend/src/auth/mod.rs` | per-handler extractor (no middleware layer); trusted-proxy + session paths |
| auth endpoints | `backend/src/handlers/auth_api.rs` | `/login` (public), `/logout` (public), `/me`; `UserPublic` type defined here |
| role split | `backend/src/handlers/alert_points_api.rs` | local `require_admin(&user)` fn (403 for viewers) — mirror this pattern |
| bootstrap admin | `backend/src/main.rs` | `BOOTSTRAP_ADMIN_EMAIL`/`_PASSWORD` env upsert at startup |
| frontend auth store | `frontend/src/lib/auth.svelte.ts` | `currentUser`, `ready`, `isAdmin`, `login()`, `logout()`, global 401 redirect |
| API client | `frontend/src/lib/api.ts` | typed fns + `ApiError`/`UnauthorizedError`/`setUnauthorizedHandler` |
| types | `frontend/src/lib/types.ts` | `UserRole`, `User`, `LoginPayload` |
| gating example | `frontend/src/routes/services/+page.svelte` | `auth.isAdmin` hides Create/Edit/Delete/Test; `editable` prop on table |

---

## 1. Decisions (recommended; ❓ = confirm)

| # | Question | Options | Recommendation |
| --- | --- | --- | --- |
| Q1 | Page path | a) `/users`, b) `/settings/users` | **a** — top-level like `/services`; matches auth-plan nav structure |
| Q2 | Visibility of the list | a) admin-only endpoints (viewers 403), b) viewers can read the list | **a** — user emails are account data; least exposure |
| Q3 | Self-deletion / self-edit rules | a) block deleting the last admin, b) also block deleting yourself | **b + a**: cannot delete the last admin, cannot delete yourself, cannot demote the last admin. Self-password change is out of scope (see §7) |
| Q4 | Editing payload | a) full form, b) partial patch | **b** — `PATCH`-style JSON body; `email`, `password` (optional = rotate), `role` all optional. Same partial-update style as `POST /alert-points/{id}/update` |
| Q5 | Password rotation on edit | a) allow admin to set new password, b) no | **a** — argon2 hash via `auth::password`; absent field = unchanged |
| Q6 | Delete mode | a) hard delete, b) soft delete (`deleted_at`) | **a** — hard delete. `alert_points` rows have no user ownership, so nothing references `users.id`; sessions for the deleted user are revoked by their Redis `user_id` index (§3) |
| Q7 | Viewers' access to `/me` | unchanged | `/auth/me` stays auth (admin or viewer) — needed for role gating everywhere |

---

## 2. Backend endpoints (admin-only)

All under `/api/v1/users`, **handler-level `CurrentUser` + `require_admin`** (mirrors
`alert_points_api.rs` — no new middleware layer):

| Method | Path | Auth | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/users` | admin (403 for viewer) | List users (id, email, role, created_at, updated_at) |
| `POST` | `/api/v1/users` | admin | Create user: `{email, password, role}` → 201 `UserPublic` |
| `POST` | `/api/v1/users/{id}/update` | admin | Partial edit: `{email?, password?, role?}` → 200 `UserPublic` |
| `POST` | `/api/v1/users/{id}/delete` | admin | Delete user → 204 |

Notes:

- State changes as POST sub-actions — same convention as `/alert-points/{id}/update|delete`.
- Email uniqueness → map `UNIQUE` violation to 409 `ApiError` (or 422 envelope — follow
  existing error style in `handlers/error.rs`); invalid role / short password / bad email
  → 422 `ApiError`.
- Role rules (Q3): reject `role: 'viewer'` on the **last admin** (demote) and reject
  `delete` on the **last admin** or **self** — 409/422 with a clear message.
  Guard = `count_admins` repo fn + `user.id == caller.id` check.
- `password` on edit (Q5): if present, hash via `auth::password::hash_password`; length
  min 8, max 256 (matches login DoS guard style).
- Sessions for a deleted/rotated user: `auth/session.rs` stores
  `sentinel:session:{token}` → `{user_id, role, email}` plus a `sentinel:session_idx:{user_id}`
  Redis SADD index (see §3); handlers DEL the index set on update/delete.
  For a role change, all live sessions of that user are revoked (their `role` is baked
  into the session payload, so an edit without revocation would let a stale session keep
  the old role until TTL).

**Wire after this change (Q7):** viewer-visible surface is unchanged —
`/auth/me` + read-only alert-points GETs.

---

## 3. Redis session revocation on update/delete

`auth/session.rs` additions (Option B store, Q1 from auth plan):

- `create()` also `SADD sentinel:session_idx:{user_id} token` (index set has the
  session TTL applied, so it self-expires).
- `revoke_user_sessions(pool, user_id)` → read index, `DEL` each `session:{token}`,
  `DEL` index set.
- `update` (email/role/password) and `delete` handlers call `revoke_user_sessions`
  after the DB write. For `email` change the session payload email is stale, so
  revocation is required there too — simplest: revoke on **any** successful update.

Self-tap check: an admin updating their own email/role is still authenticated for the
current request; their cookie's Redis key is now deleted → next request 401 → frontend
redirects to `/login` (already implemented global 401 handler in `auth.svelte.ts`).

---

## 4. Backend module layout

```text
backend/src/db/repo/users.rs     # ADD list, delete, update_email/password/role, count_admins
backend/src/auth/session.rs      # ADD revoke_user_sessions, index SADD in create
backend/src/handlers/users_api.rs  # NEW: router + handlers + require_admin (copy pattern)
backend/src/handlers/mod.rs        # ADD pub mod users_api;
backend/src/main.rs                # NEST "/api/v1/users" under the auth router block
backend/.sqlx                      # cargo sqlx prepare (SQLX_OFFLINE green)
```

`users_api.rs` types (keep in the handler, like `UserPublic` in `auth_api.rs`):

- `NewUser { email, password, role }`, `UserPatch { email?, password?, role? }`
- `UserPublic` — move from `auth_api.rs` to a shared spot (or re-export) so
  `/auth/me` and `/users` return the same shape.

Role guards: local `require_admin(user)` copied from `alert_points_api.rs` (one-line
fn, 403 `ApiError`). Last-admin/self rules live in the **handlers** (Q3), so they are
unit-testable without Redis.

---

## 5. Frontend

| File | Change |
| --- | --- |
| `frontend/src/lib/types.ts` | add `UserPublic` (reuse `User`), `NewUser { email, password, role }`, `UserPatch` |
| `frontend/src/lib/api.ts` | add `listUsers()`, `createUser(payload)`, `updateUser(id, patch)`, `deleteUser(id)` — same fetch style as alert-point fns |
| `frontend/src/lib/useUsers.svelte.ts` | NEW — `users`, `loading`, `error`; `fetch()`, `create()`, `update()`, `remove()` with optimistic update + rollback (mirror `useAlertPoints` pattern) |
| `frontend/src/routes/users/+page.svelte` | NEW — user table (email, role, created) + Create modal + role select + Edit modal (optional password field) + Delete confirm. Admin-only: `if (!auth.isAdmin) return` + redirect to `/` (viewer or unauthenticated page never renders) |
| `frontend/src/lib/components/users/` | NEW — `UsersTable`, `UserCreateModal`, `UserEditModal`, `UserDeleteConfirm`, `ErrorBanner` |
| `frontend/src/routes/+layout.svelte` | nav: **Users** link (`/users`) — hidden when `!auth.isAdmin` (mirror `/services` gating) |
| `frontend/src/lib/sentinelSocket.svelte.ts` | **no change** — cookie-auth WS rides the upgrade; role changes revoke sessions, frontend 401s and reconnects after re-login |

Delete confirm wording (Q3): "This user will be removed permanently." Last-admin /
self-delete is prevented server-side; frontend disables the button with a tooltip
explaining why (mirror the disabled reason pattern used for `/services` viewers).

---

## 6. Tests (grounded in `docs/tests.md` + existing patterns)

- `users_api` handler unit tests — pure validation: email format, password length,
  role whitelist, last-admin/self guards (no Redis).
- Session revocation — `#[ignore]` Redis test: `create` (asserts index set) →
  `revoke_user_sessions` → `get` returns None.
- `tests/api_users.rs` — **new** integration test in the existing fake-based harness
  style (`tests/api_alert_points.rs` via `backend::api::router()` + fakes):
  `ApiState` gains `users` fake + a session-store fake so login/cookie/`require_admin`
  can be driven. Cases:
  1. `GET /users` without cookie → 401; with viewer cookie → 403; with admin cookie → 200 list.
  2. `POST /users` (admin) → 201 + `UserPublic` (no password echo); duplicate email → 409.
  3. `POST /users/{id}/update` → email/role/password change; password field never returned.
  4. `POST /users/{id}/delete` → 204; self-delete → 409/422; last-admin demote → 409/422.
  5. Role change revokes sessions (session fake) — viewer with a live session sees
     the new role on next `me()`.
- `cargo build` (offline `.sqlx`), `clippy -D warnings`, `svelte-check` for frontend.

---

## 7. Explicit non-goals (deferred)

- Self-service password change / "my account" page — admins rotate passwords for now;
  a user changing their **own** password is a later task (needs `me` + old-password verify).
- Email change confirmation / password reset emails (no email service exists).
- MFA / SSO-managed users — comes via auth-plan Option D; SSO-provisioned users
  shouldn't be editable here (flagged in §8).
- User audit log of admin actions.

---

## 8. Order of work

| Step | Scope | Verify |
| --- | --- | --- |
| 1 | `db/repo/users.rs`: `list`, `delete`, `update_*`, `count_admins` + `cargo sqlx prepare` | `cargo build` offline |
| 2 | `auth/session.rs`: index SADD in `create`, `revoke_user_sessions` | `#[ignore]` Redis round-trip |
| 3 | `handlers/users_api.rs`: router + handlers + `require_admin` + last-admin/self rules | unit tests |
| 4 | Wire nest in `main.rs`; duplicate-email 409 mapping | live `curl` |
| 5 | Frontend: `types.ts` + `api.ts` fns | svelte-check |
| 6 | `useUsers.svelte.ts` + `/users` page + components + nav link | browser flow |
| 7 | `tests/api_users.rs` full pass | `cargo test` |

---

## 9. Verification plan

1. `cargo build` (SQLX_OFFLINE), `cargo clippy --all-targets -- -D warnings`, `cargo test`.
2. `pnpm check` + `pnpm build` for the frontend.
3. Manual against live stack:
   - login as admin → `/users` loads with table;
   - viewer (second account) → no nav link; direct `/users` → 403 → redirect `/`;
   - create user → appears in list; duplicate email → 409 with a clear message;
   - edit user role admin→viewer → that user's live session is revoked (next `/auth/me` → 401 → `/login`);
   - delete self / last admin → server refuses with clear message;
   - normal viewer flows unaffected (`/services` still read-only for viewers).

---

## 10. Implementation report (2026-10-10)

Everything below is implemented and verified against the live stack (shared
PostgreSQL `postgres-gs` + Valkey `valkey-gs`), with
`cargo build/clippy -D warnings/test` (offline `.sqlx`) and
`svelte-check`/`oxlint` for the frontend.

### 10.1 Backend — task 2.7

| Piece | Where | Notes / deviations |
| --- | --- | --- |
| `list`, `delete`, `update_email`, `update_password`, `update_role`, `count_admins` | `backend/src/db/repo/users.rs` | All updates return the refreshed `User` (`Option` for idempotent-miss); `count_admins` = `SELECT COUNT(*)` scalar; `.sqlx` cache regenerated with `cargo sqlx prepare` (stale entries pruned, offline build green) |
| Session index + revocation | `backend/src/auth/session.rs` | `create()` SADDs the token into `sentinel:session_idx:{user_id}` (TTL'd like the session); new `revoke_user_sessions(pool, user_id)` **and** `revoke_other_user_sessions(pool, user_id, keep_token)` — the latter keeps the current device signed in |
| Router + handlers | `backend/src/handlers/users_api.rs` (NEW) | `GET /api/v1/users`, `POST /api/v1/users` → **201 `UserAccount`**, `POST /api/v1/users/{id}/update`, `POST /api/v1/users/{id}/delete` → **204**; local `require_admin` mirrors `alert_points_api.rs` |
| Response shape | `UserAccount` | The list/table needs timestamps, so the user endpoints return `UserAccount` (`id`, `email`, `role`, `created_at`, `updated_at`) instead of re-using `UserPublic` (`/auth/me` keeps its original shape) |
| Guards (Q3) | handler-level, unit-testable without Redis | Self-delete → 409; demote/delete **last admin** → 409; **self-demote → 409** ("cannot change your own role; ask another admin" — added after live testing showed a self-demote instantly revoked the caller's own session); password rotation optional (absent = unchanged, argon2 via `auth::password`) |
| Session revocation | after **any** successful update/delete | Role/email are baked into the session payload, so stale sessions must not survive an edit; Redis failures are logged best-effort, never surfaced (mirrors the logout path) |
| Error mapping | `handlers/error.rs` + `map_user_db_err` | New `ApiError::Conflict` variant → **409** (duplicate email, rule violations); validation → **422** `Validation(field, message)`; unknown id → **404** |
| Wiring | `handlers/mod.rs`, `main.rs` | `/api/v1/users` nested under the auth router block |
| **Beyond the plan (§7)** | `handlers/auth_api.rs` | `POST /api/v1/auth/change-password` — self-service credential change (Settings page): verifies the current password, re-hashes, signs out all *other* devices, keeps the current session alive (the hash is not part of the session payload) |

### 10.2 Frontend — task 3.7

| Piece | Where |
| --- | --- |
| Types | `src/lib/types.ts`: `UserAccount` (public user + timestamps), `NewUser { email, password, role }`, `UserPatch`, `ChangePasswordPayload` |
| API fns | `src/lib/api.ts`: `listUsers()` / `createUser()` / `updateUser(id, patch)` / `deleteUser(id)` / `changePassword()` — same fetch/`ApiError`/401 style as the alert-point fns |
| `/users` page (NEW) | `src/routes/users/+page.svelte` — admin-only (`auth.ready` + `auth.isAdmin` gate, redirect to `/` otherwise); page-local `$state` with optimistic list updates, mirroring `/services` (no separate store — `useUsers.svelte.ts` dropped as unnecessary) |
| Components (NEW) | `src/lib/components/users/`: `UsersTable`, `UserCreateModal`, `UserEditModal` (email / role select / optional password reset), `UserDeleteConfirm` |
| Gating UX | table disables **Delete** on yourself with a tooltip; edit modal disables the role select for your own account (server enforces 409 anyway) |
| Nav | `routes/+layout.svelte`: **Users** link rendered only when `auth.isAdmin` (desktop + mobile rows) |
| **Settings → Account** (NEW) | `routes/settings/+page.svelte`: "Change password" form (current + new + repeat, min 8 chars, must differ) → `POST /auth/change-password`; success note "your other sessions were signed out" |
| Socket | `sentinelSocket.svelte.ts` unchanged — role/email changes revoke that user's sessions → global 401 handler → `/login` → reconnect after re-login |

### 10.3 Tests & verification

- `handlers::users_api::tests` — pure validation (email format/length, password
  bounds, 422 field pairs) + `require_admin` guard (no Redis/DB).
- `auth::session::tests` — two `#[ignore]` Redis round-trips (revocation of all
  sessions; `revoke_other_user_sessions` keeps the current token). Both green
  against live Valkey.
- Live curl smoke against the running backend (all endpoints):
  401 unauth → viewer 403 → admin 200 list → 201 create → 409 duplicate →
  422 field errors → 409 self-delete / last-admin demote / self-demote →
  role change revokes the victim's session (next `/auth/me` 401) → password
  rotation (old pass 401, new 200) → email change revokes sessions →
  change-password (wrong current 401, current session stays alive) → delete 204.
- `cargo build --all-targets`, `clippy -D warnings`, `cargo test` (offline) green;
  `svelte-check` 0 errors / 0 warnings; `oxlint src/` clean.

### 10.4 Remaining

- `tests/api_users.rs` — the fake-based integration harness (§6 cases 1–5). The
  handlers live in the binary crate against `AppState` (live pools); driving
  cookie-auth over in-memory fakes needs a session-store abstraction first, so
  the live curl smoke (§10.3) is the interim source of truth.
