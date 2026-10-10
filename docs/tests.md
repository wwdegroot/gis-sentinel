# Testing Strategy — Backend

Goal: **all unit tests run without a live PostgreSQL or Valkey/Redis instance**,
while the existing live-service smoke tests stay available behind an explicit
opt-in (`cargo` feature flag). This unblocks the work that stalled on smoke
testing last session.

> **Status: implemented (steps 1–6 of the work plan below).**
> `cargo test` passes with 49 tests and zero service dependencies;
> `cargo test --features smoke-tests -- --ignored` compiles/runs the
> live-service smoke tests. Deviations from the original plan are noted in
> §1.5 below.

### 1.5 Implementation notes / deviations (as built)

- `async-trait` added as a dependency: native `async fn` in traits is not
  dyn-compatible, so the trait methods use `#[async_trait]`.
- The Pg implementations stay in the same files as before
  (`repo/alert_points.rs` etc.) as `PgAlertPointRepository`,
  `PgProbeResultStore`, `PgActiveAlertStore` — no `pg/` subfolder needed.
- The three repo fakes come in a [`FakeStores`] bundle sharing one backing
  state (mirroring a single database) so cascade deletes span the stores;
  individual fakes remain constructible too.
- Raw sqlx query strings were kept **byte-identical** to the originals so the
  committed `.sqlx` offline cache still matches — moving them without
  preserving indentation invalidated the hashes (re-`prepare` would have
  required the live DB).

---

## 0. Current state (where we are)

| Item | State |
|---|---|
| Smoke tests for DB repo layer | ✅ `backend/tests/db_smoke.rs` — full repo round-trip, but requires live Postgres |
| Smoke tests for Redis queue | ✅ `backend/tests/queue_smoke.rs` — enqueue/dequeue round-trip, but requires live Redis/Valkey |
| Gating mechanism | ⚠️ `#[ignore = "..."]` only — tests still *compile* on every `cargo test` and can run by accident |
| Mocking support | ❌ `db/repo/*` and `queue/mod.rs` are concrete functions on `PgPool` / `RedisPool`; `ApiState` holds raw pools — no way to substitute fakes |
| Pure-logic tests (DTO validation, error envelope) | ✅ Already implemented inside `src/api/*` — not in scope here |

The blocker is **not** the smoke tests themselves — it's that the database and
queue layers cannot be mocked yet.

---

## 1. Target design

### 1.1 Trait abstraction over the storage layer

Wrap the existing SQL implementations in small traits. The production impls
keep the current `sqlx` macros verbatim — we only move them behind a trait, so
`SQLX_OFFLINE` / `.sqlx` cache behavior is unchanged.

New modules:

```
src/db/repo/
├── traits.rs        // trait definitions (new)
├── pg/              // sqlx-backed impls (code moved from current files, unchanged)
│   ├── alert_points.rs
│   ├── probe_results.rs
│   └── active_alerts.rs
└── fake.rs          // in-memory fakes for tests (new)
```

Traits (one per aggregate, matching the handler usage):

- `AlertPointRepository` — `create`, `get`, `list`, `list_filtered`,
  `count_filtered`, `update`, `delete`
- `ProbeResultStore` — `insert`, `list_recent`, `uptime_percentage`
- `ActiveAlertStore` — `insert`, `resolve`, `list_open`, `latest_open`
- `ProbeQueue` — `enqueue`, `dequeue` (blocking w/ timeout), `len`, `clear`,
  `health_check`

**Error types:** the traits must return an *own* error enum, not
`sqlx::Error` — fakes cannot construct arbitrary `sqlx::Error` variants.
Define e.g.:

```rust
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("resource not found")]
    NotFound,
    #[error("conflicting resource state")]   // e.g. second open alert
    Conflict,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}
```

`ApiError` gains `#[from] RepoError` (mapping `NotFound` → 404,
`Conflict` → 409/500, `Db` → 500) and keeps mapping `QueueError` → 503. The
existing sqlx impls then just `?`-propagate.

### 1.2 State refactor (`ApiState`)

```rust
pub struct ApiState {
    pub alert_points: Arc<dyn AlertPointRepository>,
    pub probe_results: Arc<dyn ProbeResultStore>,
    pub active_alerts: Arc<dyn ActiveAlertStore>,
    pub queue: Arc<dyn ProbeQueue>,
}
```

- `main.rs` builds the production wiring (`Pg` impls + redis pool).
- Tests build the same router with `Arc<InMemory…>` fakes.
- Handlers (`api/alert_points.rs`) switch from `repo::alert_points::create(&state.db, …)`
  calls to `state.alert_points.create(…)` — mechanical change.

### 1.3 In-memory fakes (the "mock database / redis")

Hand-written fakes (no `mockall` dependency), located in
`src/db/repo/fake.rs` / `src/queue/fake.rs`, exported from the lib crate and
gated so they are only compiled when needed:

```rust
#[cfg(any(test, feature = "test-utils"))]
pub mod fake;
```

Add to `Cargo.toml`:

```toml
[features]
smoke-tests = []
test-utils = []
```

The fakes are `Mutex<HashMap/VecDeque>`-based and must **emulate the SQL
semantics the tests rely on**:

| Semantic | Fake behavior |
|---|---|
| Unique one-open-alert-per-point | `insert` returns `RepoError::Conflict` if an open alert exists for the point |
| Cascade delete | `delete(alert_point)` removes its probe results + active alerts |
| Snippet truncation | `insert_probe_result` truncates `raw_response_snippet` to 2000 chars |
| `ILIKE` search + filters + pagination | substring match, exact-match filters, limit/offset |
| `uptime_percentage` | computed from stored probe results over the window |
| Queue FIFO + `BRPOP` timeout | `VecDeque` pop with tokio timeout returning `None` on empty |
| Error injection | fakes expose a `fail_next(error)` hook so tests can drive 500 / 503 paths |

> Rationale for emulating SQL semantics in the fakes: the smoke tests
> (`db_smoke.rs`, `queue_smoke.rs`) already assert these behaviors against the
> real Postgres/Redis. The unit tests reuse the same scenario logic against the
> fakes; the smoke tests remain the source of truth that the *sqlx/redis
> impls* match the fakes' semantics.

### 1.4 Smoke tests behind a cargo feature

Use `required-features` on the test targets — cleanest option, excludes the
files from compilation entirely and doesn't need `#[cfg]` soup:

```toml
[[test]]
name = "db_smoke"
required-features = ["smoke-tests"]

[[test]]
name = "queue_smoke"
required-features = ["smoke-tests"]
```

- `cargo test` → compiles & runs unit tests only (no Postgres/Redis needed).
- `cargo test --features smoke-tests -- --ignored` → compiles & runs the smoke
  tests against live services (DATABASE_URL / REDIS_URL from `.env`).
- Keep the `#[ignore = "..."]` attributes as a second guard (protects against
  running `cargo test --features smoke-tests` without live services).

CI mapping (future task 4.2): `cargo test` in the default job; the smoke
feature job runs only when a Postgres/Redis service container is attached.

---

## 2. Test files to create

| File | Layer | What it covers |
|---|---|---|
| `src/queue/mod.rs` (unit tests, `#[cfg(test)]`) | queue | `ProbeJob` serde round-trip, key/channel constants stable |
| `src/queue/fake.rs` (unit tests) | queue | enqueue → dequeue FIFO order, `len`, `clear`, blocking dequeue returns `None` on timeout |
| `src/db/repo/fake.rs` (unit tests) | db | create/get/update/delete lifecycle, one-open-alert conflict, cascade delete, snippet truncation, filters + pagination, uptime calculation |
| `tests/api_alert_points.rs` | API handlers | Router built from `ApiState` over fakes, via `tower::ServiceExt::oneshot` / `axum::serve` on ephemeral port: CRUD happy paths, validation errors (400 + details envelope), 404s, pagination/filter query params, `POST /test` → 202 + job visible in queue fake, queue failure → 503, DB failure injection → 500 |

The API-handler integration tests are the main win: they exercise the full
router + extractors + error envelope + validation without any I/O dependency.

---

## 3. Work plan (ordered)

1. **Error types** — add `RepoError`; extend `ApiError` mapping. *(no behavior change)*
2. **Traits** — `src/db/repo/traits.rs` + `src/queue` trait; move current sqlx
   function bodies into `pg/` impl structs (code unchanged). Update handlers
   and `main.rs` wiring to use `Arc<dyn …>` in `ApiState`.
   ✅ Acceptance: `cargo check && cargo build` green, `.sqlx` offline cache still valid,
   app behaves identically.
3. **Feature flags** — add `smoke-tests` / `test-utils` features + `[[test]]`
   `required-features` entries. ✅ `cargo test` (no services running) passes
   without compiling the smoke tests.
4. **Fakes** — implement in-memory fakes incl. SQL-semantics emulation and
   failure injection; export under `test-utils`.
5. **Unit tests** — fake-level tests for repo semantics and queue semantics.
6. **API handler tests** — router-over-fakes integration tests per the table
   above.
7. **Update `docs/tasks.md`** — check off 5.1 items ("Mock tests for Redis
   queue…", "Integration tests for REST CRUD…") and reference this document.

Steps 1–3 are pure refactoring with zero test addition; 4–6 add the actual
tests. Nothing in the plan touches the live database.

---

## 4. Out of scope / deferred

- `testcontainers` or any Docker-dependent integration testing — smoke tests
  against the shared `postgres-gs` / `valkey-gs` containers cover that.
- `mockall` — hand-written fakes suffice at this codebase size.
- Frontend tests (Phase 5.2) — separate effort.
- Worker/scheduler tests (task 2.3) — the `ProbeQueue` trait designed here is
  exactly what the worker will consume, so its tests will reuse the same fakes.
