# Phase 2 — Backend Services Implementation (Plans & Implementation Reports)

Covers `tasks.md` → Phase 2.

| Part | Task | Status |
|---|---|---|
| Part 1 | 2.1 API Service (Alert Points CRUD) | ✅ **Implemented** (report in Part 1 §10) |
| Part 2 | 2.2 Scheduler (Queue Producer) | 📝 **PLANNED** — see Part 2 |
| Part 3 | 2.3 Worker (Probe Executor & Evaluator) | 📝 **PLANNED** — see Part 3 |
| Part 4 | 2.4 WebSocket & Real-time Alert Hub | 📝 **PLANNED** — see Part 4 |
| Part 5 | 2.5 Backend Technical Debt | 📝 **PLANNED** — see Part 5 |

Testing infrastructure already exists from `docs/tests.md`: repository/queue
class traits + in-memory fakes (`test-utils` feature), so scheduler/worker/hub
logic is testable without live services; live-service coverage sits behind the
`smoke-tests` feature.

---

## Part 1 — Task 2.1 API Service (Alert Points CRUD)

Status: ✅ **IMPLEMENTED** — see report in section 10.

### 0. Key Decisions (confirmed)

| # | Question | Options | Decision |
|---|----------|---------|----------|
| Q1 | Scope of phase2.md / this step | a) backend API only, b) also build the frontend `/services` page (task 3.4) | **a) backend only** — frontend arrives with Phase 3 |
| Q2 | Payload validation approach | a) `validator` crate with derive rules, b) hand-rolled `validate()` functions | **a) `validator` crate (derive)** |
| Q3 | `POST /:id/test` behavior while no worker exists (task 2.3) | a) synchronously probe now, b) enqueue a `ProbeJob` (consumed later by 2.3), c) enqueue + 202 | **b) enqueue a `ProbeJob`** onto the Valkey queue via existing `queue::enqueue_probe_job`; returns `202 Accepted` with the enqueued job echo |
| Q4 | `PUT /:id` semantics | a) partial update (only provided fields change), b) strict full replacement | **a) partial update** — matches the already-implemented `repo::alert_points::update()` `COALESCE` design |
| Q5 | Error-handling crates | a) hand-rolled `ApiError` enum, b) `thiserror` (typed derive-based errors), c) `anyhow` (type-erased) | **b + c, each in its natural habitat**: `thiserror` for all *typed* error enums — the new `ApiError` and a refactor of the existing hand-rolled `QueueError` (variant matching drives 404-vs-400-vs-503 mapping; `?` + `From` impls remove handler boilerplate). `anyhow` for *application* bootstrap in `main.rs` (replaces the `panic!`/`unwrap` chains; type erasure is fine where the process just exits). `anyhow` is **not** used in library/route code: it erases the concrete type, making status-code routing impossible |

Additional assumptions (documented, not blocking):

- **Probe timeout source**: `alert_points` has no `timeout` column, but `ProbeJob.timeout_ms` requires one. Use a constant default of **10 000 ms** for now (module-level `const DEFAULT_PROBE_TIMEOUT_MS: i64`); making it configurable is deferred until task 2.3 needs it.
- **Mock data stays**: the demo `active_alerts` in `main.rs` and legacy `ws_handler` are cleaned up in task 2.4, not here.

### 1. Dependencies (`backend/Cargo.toml`)

```toml
validator = { version = "0.19", features = ["derive"] }
thiserror = "2"
anyhow = "1"
```

(`serde`, `serde_json`, `uuid`, `chrono`, `sqlx`, `axum` already present from Phase 1; `tracing` already present for logging swallowed `500`-cause errors.)

### 1.1 Error-Handling Refactor (existing code)

Two existing error surfaces get modernized *before* the API is built on top of them:

**a) `QueueError` → `thiserror` derive (`src/queue/mod.rs`)**

Replace the ~35 lines of manual `Display`, `std::error::Error`, and `From` impls (lines 32–65) with:

```rust
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("redis queue error: {0}")]
    Pool(#[from] deadpool_redis::PoolError),
    #[error("invalid job payload: {0}")]
    Serde(#[from] serde_json::Error),
}

// Keep manual: thiserror cannot do transitive From (RedisError -> PoolError -> QueueError).
impl From<redis::RedisError> for QueueError {
    fn from(e: redis::RedisError) -> Self {
        QueueError::Pool(e.into())
    }
}
```

- Behavior-preserving: same variant names, same `Display` messages → `tests/queue_smoke.rs` compiles unchanged.
- Improvement over the current code: `#[from]` wires proper `source()` chaining (the current `impl std::error::Error for QueueError {}` drops the inner cause), which `ApiError::QueueUnavailable(#[source] QueueError)` and `tracing::error!` logging rely on later.
- Public API of the queue module unchanged; all `Result<_, QueueError>` signatures stay.

**b) `main.rs` bootstrap → `anyhow`**

Replace the five `unwrap_or_else(|e| panic!(...))` / `.unwrap()` chains (config load, DB connect, migrations, redis pool, serve) with:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()
        .context("invalid configuration")?;
    let db_pool = db::connect(&config.database_url)
        .await
        .context("could not connect to database")?;
    db::run_migrations(&db_pool).await.context("database migrations failed")?;
    // ...
    axum::serve(listener, app).await.context("server error")?;
    Ok(())
}
```

- Same failure behavior as today (process exits with an error), but anyhow prints a clean `Error: <msg>` with the full cause chain instead of a panic backtrace.
- Scope guard: `anyhow` is used **only** in `main.rs`; all `src/` modules and handlers keep typed errors.

After code changes: refresh the offline query cache with `cargo sqlx prepare` (run from `backend/`, `DATABASE_URL` pointing at `postgres-gs:5432`, `SQLX_OFFLINE=true` keeps builds reproducible).

### 2. API Surface — REST routes under `/api/v1/alert-points`

| Method | Path | Success | Errors | Description |
|---|---|---|---|---|
| `GET` | `/api/v1/alert-points` | `200` + page envelope | `400` (bad query params) | List with filtering & pagination |
| `POST` | `/api/v1/alert-points` | `201` + created `AlertPoint` | `400`/`422` (validation) | Create a monitoring point |
| `GET` | `/api/v1/alert-points/:id` | `200` + detail w/ history | `404` | Details + recent probe history |
| `PUT` | `/api/v1/alert-points/:id` | `200` + updated `AlertPoint` | `400`/`422`, `404` | Partial update of settings |
| `DELETE` | `/api/v1/alert-points/:id` | `204` no content | `404` | Delete (probe history cascades) |
| `POST` | `/api/v1/alert-points/:id/test` | `202` + `{ queued: true, job }` | `404`, `503` (queue down) | On-demand probe: enqueue `ProbeJob` |

Note: `422` is returned by the `Json` extractor itself for bodies that fail deserialization (e.g. unknown `service_type`); `400` is returned for structurally valid bodies that fail `validator` rules.

### 3. New Module Layout

```
backend/src/
├── api/
│   ├── mod.rs          # api_router(state) -> Router<AppState>, error type, pagination types
│   └── alert_points.rs # route handlers for the six endpoints
├── handlers/mod.rs     # unchanged this step (ws + static handlers stay)
```

`main.rs` mounts the API via `.nest("/api/v1", api::api_router(...))` (or `.merge` with full paths) alongside the existing `/ws` routes and static fallback.

**AppState changes**: `db: PgPool` and `redis: RedisPool` become `pub(crate)` (currently `#[allow(dead_code)]` private) so API handlers can use them. The mock `active_alerts`/broadcast fields remain untouched (cleanup = 2.4).

### 4. Request / Response Contracts

New DTOs in `backend/src/api/alert_points.rs` (or a small `api/dto.rs`), reusing the DB enums `ServiceType` / `HttpMethod` from `db::models` (serde already serializes them as `WMS`, `WFS`, ..., `ArcGIS_REST`, `HTTP`, `GET`, `POST` — the wire contract from `tasks.md`):

**`POST /api/v1/alert-points` request** — `CreateAlertPointRequest` (wraps/mirrors `db::models::NewAlertPoint` plus validation attributes):

| Field | Type | Validation |
|---|---|---|
| `name` | `String` | `length(1..=200)`, trimmed non-empty |
| `url` | `String` | valid `http`/`https` URL (validator `url` + scheme check) |
| `service_type` | `ServiceType` | deserialization rejects unknown values |
| `check_interval_seconds` | `i32` | `range(1..=86_400)` |
| `expected_response_time_ms` | `i32` | `range(1..=600_000)` |
| `http_method` | `HttpMethod` | optional, default `GET` |
| `custom_headers` | `Value` | optional, default `{}`; must be a JSON **object** whose values are strings |
| `auth_config` | `Option<Value>` | optional, if present must be a JSON object |
| `enabled` | `bool` | optional, default `true` |

**`PUT /api/v1/alert-points/:id` request** — `UpdateAlertPointRequest` (mirrors `db::models::UpdateAlertPoint`): all fields `Option`; validation applies **only to `Some` values** (validator `#[validation(...)]` on nested Option fields or manual cross-checks in a `validate_partial` helper). `auth_config` keeps the `Option<Option<Value>>` three-state convention already implemented in the repo layer (absent = unchanged, `null` = cleared, value = set).

**`GET /api/v1/alert-points` query parameters** (`ListAlertPointsQuery`):

| Param | Type | Default | Notes |
|---|---|---|---|
| `page` | `u32` | `1` | 1-based |
| `page_size` | `u32` | `20` | clamped to `1..=100` |
| `enabled` | `Option<bool>` | — | filter on `enabled` |
| `service_type` | `Option<ServiceType>` | — | filter on type |
| `search` | `Option<String>` | — | case-insensitive `ILIKE` on `name`/`url` |

**`200` response envelope**:

```json
{
  "data": [ AlertPoint... ],
  "pagination": { "page": 1, "page_size": 20, "total": 42, "total_pages": 3 }
}
```

**`GET /api/v1/alert-points/:id` response** — `AlertPointDetail`:

```json
{
  "alert_point": { ...AlertPoint... },
  "recent_results": [ ...last 20 probe_results, newest first... ],
  "uptime_24h_pct": 99.5,          // null if no results
  "open_alert": { ...ActiveAlert... } // null if none
}
```

**Error envelope** (all endpoints), via an `ApiError` enum defined with `#[derive(Debug, thiserror::Error)]` and implementing `IntoResponse`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("validation failed")]
    Validation(HashMap<String, Vec<String>>),   // -> 400 + field details
    #[error("{0}")]
    BadRequest(String),                         // -> 400
    #[error("resource not found")]
    NotFound,                                   // -> 404
    #[error("database error")]
    Database(#[from] sqlx::Error),              // -> 500 (cause logged, not leaked)
    #[error("queue unavailable")]
    QueueUnavailable(#[source] QueueError),     // -> 503
}
```

- `#[from] sqlx::Error` lets handlers use `repo::get(...)?` directly; `IntoResponse` inspects the sqlx error kind to translate `RowNotFound` into `404` where appropriate.
- `IntoResponse` renders every variant into the uniform JSON envelope below; `500`/`503` variants log the source chain with `tracing::error!` while responding with a generic message (no internals leaked).
- The hand-written error envelope remains:

```json
{ "error": { "code": "validation", "message": "...", "details": { "field": ["msg"] } } }
```

### 5. Repository Additions (`db/repo/alert_points.rs`, `db/repo/active_alerts.rs`)

- `list_filtered(pool, filter: ListFilter { enabled: Option<bool>, service_type: Option<ServiceType>, search: Option<String>, limit: i64, offset: i64 }) -> Vec<AlertPoint>` — dynamic `WHERE` via compile-time-checked query with `COALESCE`-style optional params (keeps `SQLX_OFFLINE` compatible; no query-building DSL needed).
- `count_filtered(pool, same filter minus limit/offset) -> i64` — for the pagination envelope.
- Existing `create` / `get` / `update` / `delete` are reused as-is.
- `active_alerts` repo: add `get_open_for_point(pool, alert_point_id) -> Option<ActiveAlert>` (SELECT ... `WHERE alert_point_id = $1 AND resolved_at IS NULL`) for the detail endpoint.
- `probe_results::list_recent` and `uptime_percentage` already exist and are reused.

### 6. Handler Logic

- **create**: `Json<CreateAlertPointRequest>` → `.validate()` → map to `NewAlertPoint` → `repo::create` → `201`.
- **list**: `Query<ListAlertPointsQuery>` (validated/clamped) → `list_filtered` + `count_filtered` → envelope.
- **get**: `Path<Uuid>` → `repo::get` → join in recent results, uptime, open alert → `AlertPointDetail`.
- **update**: `Path` + `Json<UpdateAlertPointRequest>` → validate `Some` fields → `repo::update` → `200`; `RowNotFound` → `404`.
- **delete**: `repo::delete` → `204` if `true`, `404` if `false`.
- **test**: `repo::get` → build `ProbeJob { target_id: id, url, service_type, expected_time_ms, timeout_ms: DEFAULT_PROBE_TIMEOUT_MS }` → `queue::enqueue_probe_job(&state.redis, &job)` → `202 { "queued": true, "job": {...} }`; queue pool errors flow through `ApiError::QueueUnavailable` → `503 QueueUnavailable` (per Phase 1 the server tolerates Valkey being down at boot).
- **Handler ergonomics**: with `thiserror` `From` impls in place, handler bodies return `Result<impl IntoResponse, ApiError>` and propagate with `?` — no manual `match` on repo/queue results except for intentional translations (e.g. `delete` returning `bool` → `NotFound`).

### 7. Testing

- **Unit tests** (`#[cfg(test)]` in the API module): validation rules for `CreateAlertPointRequest`/`UpdateAlertPointRequest` (bad URL, bad scheme `ftp://`, zero/negative interval, empty name, non-object `custom_headers`, valid full payload), query-param clamping, and pagination math.
- **Integration tests** (`backend/tests/api_smoke.rs`, pattern of existing `db_smoke.rs`): build the router with a real pool against `postgres-gs`, exercise full CRUD lifecycle + `test` endpoint enqueue (requires Valkey in `valkey-gs`): create → 201, get → 200 with history fields, list with filters/pagination, put → partial update, delete → 204 then get → 404, test → 202 and job visible in `sentinel:probe_jobs` (then clean the queue).
- **Manual smoke**: `curl` against a locally running backend.

### 8. Verification Checklist (definition of done for 2.1)

- [x] `validator` + `thiserror` + `anyhow` dependencies added; all request DTOs carry validation rules
- [x] `QueueError` refactored to `thiserror` derive, behavior-preserving (`queue_smoke` green); `main.rs` bootstrap on `anyhow::Result` with `.context(...)`, `panic!` chains removed
- [x] `ApiError` defined via `thiserror` derive; conversions used so handlers propagate with `?`; `500`/`503` sources logged via `tracing` *(deviation: `sqlx::Error` wrapped in `RepoError` per `docs/tests.md` trait refactor)*
- [x] `GET  /api/v1/alert-points` with `page`, `page_size`, `enabled`, `service_type`, `search` + pagination envelope
- [x] `POST /api/v1/alert-points` → `201`; invalid payloads → `400` with field-level details; unknown `service_type` → `400 bad_request` *(deviation: serde rejections mapped to uniform `400`, not `422`)*
- [x] `GET  /api/v1/alert-points/:id` → detail incl. recent probe history, uptime %, open alert; unknown id → `404`
- [x] `PUT  /api/v1/alert-points/:id` → partial update; absent fields unchanged; `auth_config` 3-state works
- [x] `DELETE /api/v1/alert-points/:id` → `204`; cascades verified; unknown id → `404`
- [x] `POST /api/v1/alert-points/:id/test` → `202`, `ProbeJob` verifiable in the queue *(live Redis check = smoke test; unit-verified via `FakeProbeQueue`)*
- [x] Uniform JSON error envelope for all error paths
- [x] Handlers reach DB/queue via state *(deviation: `ApiState` holds `Arc<dyn repo/queue traits>` instead of raw pools — enables the mocked tests)*
- [x] Repo: `list_filtered` + `count_filtered` + `active_alerts::latest_open` *(renamed from `get_open_for_point`)*
- [x] `cargo sqlx prepare` refresh **not needed** — raw SQL kept byte-identical through the trait refactor, offline cache still valid
- [x] `cargo build` + `cargo test` green (31 lib + 18 integration tests, zero live services required)
- [x] `docs/tasks.md` checkbox 2.1 ticked; this file updated from *Planned* → *Implemented* with a report (below)

### 9. Implementation Order

1. **Error-handling refactor**: add `validator` + `thiserror` + `anyhow` deps; convert `QueueError` to `thiserror` derive; convert `main.rs` bootstrap to `anyhow::Result`; verify `queue_smoke` still passes
2. Create `api/mod.rs` with the `thiserror`-derived `ApiError` (`IntoResponse` impl, source logging, rejection mapping); make `AppState` fields `pub(crate)`
3. DTOs + validation + unit tests
4. Repo additions (`list_filtered`, `count_filtered`, `get_open_for_point`) + `cargo sqlx prepare`
5. Handlers for the six routes + wire router in `main.rs`
6. Integration test `api_smoke.rs`; run against `postgres-gs` / `valkey-gs`
7. `cargo build` + full test pass; docs update (`tasks.md` + this file)

### 10. Implementation Report

Implemented per plan with the `docs/tests.md` testing infrastructure. Deviations:

1. **State**: `ApiState` holds `Arc<dyn AlertPointRepository / ProbeResultStore / ActiveAlertStore / ProbeQueue>` instead of raw pools — required for the mocked test suite.
2. **Errors**: `ApiError::Repository(RepoError)` replaces the planned `From<sqlx::Error>`; `RepoError::NotFound → 404`, `Conflict (second open alert) → 409`, `Db → 500`.
3. **Validation status codes**: unknown `service_type` is a serde rejection mapped to `400 bad_request` (not `422`) — uniform envelope everywhere.
4. **Routes**: the `/api/v1` prefix is baked into `api::router()` (single source of truth). An initial drift (routes merged at root) was caught and fixed.
5. **Tests**: `tests/api_alert_points.rs` (18 tests) runs the full router against in-memory fakes via the `test-utils` feature instead of a live `api_smoke.rs`; live-service semantics remain covered by the feature-gated smoke tests (`smoke-tests`).
6. **`cargo sqlx prepare`**: not needed — raw SQL strings were preserved byte-identically through the refactor.

Final state: 49 tests green (31 lib + 18 integration), zero live services required for `cargo test`.

---

## Part 2 — Task 2.2 Scheduler Service (Queue Producer)

Status: 📝 **PLANNED**

### Decisions (confirmed)

| # | Question | Decision |
|---|----------|----------|
| Q1 | Due-time bookkeeping | **Single tick loop** (`tokio::time::interval`, 1s) + in-memory `next_due` map (`target_id → Instant`). Periodic resync from DB + immediate resync on CRUD changes |
| Q2 | Duplicate in-flight checks | **Redis lock**: `SET sentinel:inflight:{target_id} <node-id> NX EX <interval + probe_timeout>` — guarantees no duplicate in-flight probes even with queue backlog or multiple scheduler instances; TTL is the crash safety net |
| Q3 | First check of a new point | **Immediately due** (`next_due = now` on resync-add) — fast feedback after creation; configurable later if needed |
| Q4 | Scheduler on/off for dev | `SCHEDULER_ENABLED` env (default `true`) so API-only dev/test runs don't enqueue probes |

### Design

New module `backend/src/scheduler/mod.rs`:

```text
SchedulerDeps {
    alert_points: Arc<dyn AlertPointRepository>,
    queue:       Arc<dyn ProbeQueue>,
    inflight:    Arc<dyn InFlightGuard>,
    resync:      Arc<Notify>,          // poked by API mutations
}

loop {                                  // per tick (1s)
    every RESYNC_SECS or on notify:
        resync()   // fetch enabled points; add new (next_due = now),
                   // remove disabled/deleted, refresh intervals/urls
    for target where next_due <= now:
        if !inflight.try_acquire(target_id, ttl = interval + timeout): continue  // skip, keep due
        match queue.enqueue_probe_job(&job_for_point(point)) {
            Ok(_)  => next_due[target] = now + interval
            Err(e) => { inflight.release(target_id); log; /* retry next tick */ }
        }
}
```

- **`InFlightGuard` trait** (in `queue/`): `try_acquire(target_id, ttl) -> bool`, `release(target_id)`. `RedisInFlightGuard` uses `SET … NX EX`; an `InMemoryGuard` fake ships under `test-utils`.
- **Worker cooperation (2.3)**: the worker `DEL`s the in-flight key when a probe completes, so the next interval fires on time; the TTL only covers crashed workers.
- **Shared job builder**: extract `queue::job_for_point(&AlertPoint) -> ProbeJob` (used by both the `POST /{id}/test` handler and the scheduler; keeps `DEFAULT_PROBE_TIMEOUT_MS` single-sourced).
- **CRUD coupling**: `ApiState` gains `resync: Arc<tokio::sync::Notify>`; `create/update/delete` handlers call `resync.notify_one()`. The scheduler also resyncs every `SCHEDULER_RESYNC_SECS` (default 30) as a safety net.
- **Config additions** (`Config` + `.env.example`): `SCHEDULER_ENABLED` (true), `SCHEDULER_TICK_MS` (1000), `SCHEDULER_RESYNC_SECS` (30).
- **Wiring**: `main.rs` spawns the task after pools are built; shutdown-aware (Part 5). Uses the same trait objects as `ApiState`.
- Missed-time policy: a `next_due` far in the past (server downtime) simply means "due now" — one immediate check, no backlog spam.

### Testing

- **Unit (fakes, no services)**: due-target selection; lock prevents duplicate enqueue; lock-conflict skips without advancing `next_due`; enqueue failure releases lock + retries; resync adds/removes/reset intervals; disabled points never scheduled.
- **Smoke (behind `smoke-tests`)**: live DB+Valkey — create point with 1s interval → job appears in `sentinel:probe_jobs` within a tick; in-flight key present; duplicate tick does not double-enqueue while the lock is held.

### Definition of done

- [ ] `scheduler` module with tick loop + due map + resync (periodic + notify)
- [ ] `InFlightGuard` trait + Redis impl + in-memory fake (exported under `test-utils`)
- [ ] `job_for_point` shared helper; `POST /{id}/test` refactored onto it
- [ ] `ApiState.resync` notify wired from create/update/delete handlers
- [ ] Config envs + `.env.example` updated
- [ ] Unit tests over fakes green; smoke test green with live services
- [ ] `docs/tasks.md` 2.2 ticked; this part updated to *Implemented*

### Implementation order

1. `job_for_point` extraction + `InFlightGuard` trait/impls
2. Scheduler tick loop + resync logic (pure, over fakes) + unit tests
3. `main.rs` wiring + config + notify coupling in API handlers
4. Smoke test + docs update

---

## Part 3 — Task 2.3 Worker Service (GIS Probe Executor & Evaluator)

Status: 📝 **PLANNED**

### Decisions (confirmed)

| # | Question | Decision |
|---|----------|----------|
| Q1 | Probe depth | **HTTP status + content sniff** — reqwest GET/POST with headers/auth/timeout; sniff body for service signatures (`WMS_Capabilities`, `FeatureCollection`, ArcGIS `json.Folder`, …). No XML parser dep. Capability-level parsing = optional follow-up (2.3b) |
| Q2 | Signature mismatch on 2xx | Treated as **Down** (`"unexpected response content"`) — the point exists to monitor a specific GIS service |
| Q3 | Status model | **Derived, no schema change**: `Healthy` = 2xx + signature + latency ≤ expected; `Degraded` = up but slow; `Down` = error / 4xx / 5xx / mismatch / timeout |
| Q4 | Concurrency | `WORKER_CONCURRENCY` env, default **4** |
| Q5 | Latency measurement | Total request duration via `Instant` (TTFB split deferred — no consumer yet) |
| Q6 | Auth convention | `auth_config`: `{"kind":"basic","username","password"}` or `{"kind":"bearer","token"}`; ad-hoc tokens go through `custom_headers` |

### Alert state machine (pure function, `evaluate.rs`)

| Probe result | Open alert? | Action |
|---|---|---|
| Down | none | insert `New` alert (reason = status/error) + publish `New` |
| Down | open, same reason | no-op (no event spam) |
| Down | open, different reason | publish `Update` (row reason unchanged = first cause) |
| Degraded | none | insert `New` (`"slow: {ms} ms > {expected} ms"`) + publish |
| Degraded | open (Down) | publish `Update` |
| Healthy | open | `resolve` + publish `Remove` |
| Healthy | none | no-op |

### Design

```text
src/worker/
├── mod.rs       # N worker loops: BRPOP(timeout) → probe → persist → evaluate → publish → release lock
├── probe.rs     # ProbeExecutor trait + ReqwestProbeExecutor + per-type signature sniffing
└── evaluate.rs  # evaluate(outcome, open_alert) -> Option<Action>; AlertEvent building
```

- **`ProbeExecutor` trait**: `async fn probe(&self, job, auth, headers) -> ProbeOutcome` — `ReqwestProbeExecutor` in prod; a fake drives tests. `ProbeOutcome { status: Healthy{ms}|Degraded{ms}|Down{reason}, status_code, error_message, snippet }`.
- **Probe request**: URL/method/headers from the alert point (fetched by id — `ProbeJob` wire contract stays unchanged); timeout = `job.timeout_ms`; timeout → `Down`.
- **Persistence**: every probe inserts a `probe_results` row (store truncates snippets); alert lifecycle via `ActiveAlertStore`.
- **Events**: worker publishes `AlertEvent` to `sentinel:alerts` via a new **`AlertBus`** trait (Redis `PUBLISH` impl + in-memory fake under `test-utils`). Payload keeps the frontend-compatible `SentinelAlert` shape (`{id, name, atype, performance, expected, up, reason, error}`) extended with `point_id` + `status` (`Down|Degraded`); `id` = active-alert id (stable across `New`→`Update`→`Remove` so the frontend can key on it). Frontend adopts `status`/`point_id` in task 3.1.
- **In-flight release**: worker `DEL`s `sentinel:inflight:{target_id}` after completion (scheduler's TTL is only the crash net).
- **Graceful shutdown** (Part 5): workers poll with short timeout, check the cancellation token between jobs, drain in-flight work.
- **Deps**: `reqwest` (`default-features = false`, `rustls-tls`, `json`).

### Testing

- **`evaluate()` truth-table unit tests** — every row of the transition matrix, over fakes.
- **Signature sniffing unit tests** — per `ServiceType`, canned bodies (WMS Capabilities XML, WFS, WMTS, OAF FeatureCollection, ArcGIS JSON, HTML junk).
- **Probe executor tests against a local axum stub server** (ephemeral port: 200+XML, 500, slow response, wrong content, timeout) — self-contained, no external services or Docker.
- **Worker loop test** — `FakeProbeQueue` + `FakeStores` + in-memory `AlertBus`: enqueue job → assert probe row, alert transition, published event, lock released.
- **Smoke (behind `smoke-tests`)**: full loop with live Valkey + local stub server.

### Definition of done

- [ ] `ProbeExecutor` trait + reqwest impl + sniffing; `AlertBus` trait + Redis/in-memory impls
- [ ] `evaluate()` transition matrix + unit tests
- [ ] Worker pool (N loops) with persistence, publishing, lock release
- [ ] `WORKER_CONCURRENCY` config + `.env.example`
- [ ] All unit tests green without services; smoke test green with live services
- [ ] `docs/tasks.md` 2.3 ticked; this part updated to *Implemented*

### Implementation order

1. `ProbeExecutor` + sniffing (+ stub-server tests)
2. `evaluate()` + truth-table tests
3. `AlertBus` trait/impls; worker loop wiring persistence + events + lock release
4. Config, smoke test, docs update

---

## Part 4 — Task 2.4 WebSocket & Real-time Alert Hub

Status: 📝 **PLANNED**

### Decisions (confirmed)

| # | Question | Decision |
|---|----------|----------|
| Q1 | Transport | **Redis Pub/Sub from the start**: worker → `PUBLISH sentinel:alerts` → hub task → tokio broadcast → WS clients. Multi-instance-safe; pub/sub reconnect handled in the hub |
| Q2 | Initial state sync | On connect, send full open-alert list composed from DB (open alerts + point names + latest probe result for the latency value), then stream incremental events |
| Q3 | Wire contract | Keep existing `SentinelAlert` field names (`id`, `name`, `atype`, `performance`, `expected`, `up`, `reason`, `error`) for frontend compatibility; `Update`/`Remove` handling + renames are task 3.1 |

### Design

- **Hub task** (`src/handlers/alert_hub.rs`): dedicated Redis pub/sub connection — note `deadpool-redis` does **not** support pub/sub; use `redis::Client::get_async_pubsub()` directly. Subscribe → deserialize `AlertEvent` → `tokio::sync::broadcast::Sender<AlertEvent>` (channel size 64; lag logged).
- **Reconnect loop**: on pub/sub connection loss, retry with exponential backoff; WS clients keep receiving (broadcast channel bridges the gap briefly).
- **WS handler** (`sentinel_ws.rs` rewritten): upgrade → initial sync (queries: `active_alerts.list_open` + point names + `probe_results.list_recent(limit 1)` per point — N+1 acceptable at current scale, noted) → split socket → forward broadcast events → ping/keepalive → clean close handling.
- **Legacy cleanup (from tasks.md 2.4)**: delete `handlers/websockets.rs` (echo demo), delete mock `active_alerts`/broadcast bootstrap in `main.rs`; `AppState` shrinks to the broadcast handle (or merges into `ApiState`); consolidate routes under `/ws/sentinel`.
- Redis channel constant `sentinel:alerts` already exists in `queue/mod.rs`.

### Testing

- **Unit**: `AlertEvent` serde round-trip; initial-sync composition over `FakeStores`.
- **Integration (no services)**: WS client (`tokio-tungstenite`, already a dep) against the router with fakes — connect → receive initial state; trigger fake event → receive it.
- **Smoke (behind `smoke-tests`)**: live pub/sub end-to-end — worker publishes → WS client receives.

### Definition of done

- [ ] Hub task with pub/sub subscribe + reconnect + broadcast fan-out
- [ ] Rewritten `sentinel_ws.rs` with initial DB sync; echo/legacy demo removed
- [ ] `main.rs` mock data + legacy routes cleaned up; routes consolidated
- [ ] Wire contract documented (payload fields + `New`/`Update`/`Remove` semantics)
- [ ] Unit + no-service integration tests green; smoke test green
- [ ] `docs/tasks.md` 2.4 ticked; this part updated to *Implemented*

### Implementation order

1. `AlertEvent` payload types + serde tests
2. Hub task (pub/sub → broadcast) with reconnect
3. WS handler rewrite + initial sync
4. Legacy cleanup + integration tests + docs update

---

## Part 5 — Task 2.5 Backend Technical Debt & Fixes

Status: 📝 **PLANNED**

### Scope

| Item | Approach |
|---|---|
| Graceful shutdown | `axum::serve(..).with_graceful_shutdown(ctrl_c)`; a `CancellationToken` stops scheduler tick, worker polls (drain in-flight probes), and the hub; then pools close. Env `SHUTDOWN_TIMEOUT_SECS` (default 30) as hard cap |
| Request tracing IDs | `tower-http` `RequestIdLayer` + `PropagateRequestIdLayer` (`x-request-id`); id added to the tracing span so logs correlate per request |
| Config hygiene | All new envs (`SCHEDULER_*`, `WORKER_CONCURRENCY`, `SHUTDOWN_TIMEOUT_SECS`) into `Config` + `.env.example` |
| sqlx cache | Worker/scheduler reuse existing repo methods — no new queries expected; run `cargo sqlx prepare` only if queries are added |

### Definition of done

- [ ] Ctrl-C → clean shutdown: server stops accepting, in-flight probes finish, tasks joined, process exits 0
- [ ] `x-request-id` on responses + in log lines
- [ ] `.env.example` documents all envs
- [ ] `cargo build` + full test suite green
- [ ] `docs/tasks.md` 2.5 ticked

### Cross-part implementation order

**Part 2 (scheduler) → Part 3 (worker) → Part 4 (hub) → Part 5 (debt).** Parts 2+3 are co-developed (scheduler's output is the worker's input; both are unit-tested over fakes before live wiring). Shutdown support from Part 5 lands incrementally — each spawned task accepts the cancellation token from day one, so Part 5 is mostly wiring + polish.
