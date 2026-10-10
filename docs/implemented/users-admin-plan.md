# User Admin Plan — manage users (admin-only CRUD UI + endpoints)

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
