# Phase 2 — Backend Services Implementation (Change Plan)

Covers `tasks.md` → Phase 2: **2.1 API Service (Alert Points CRUD)** · **2.2 Scheduler (Queue Producer)** · **2.3 Worker (Probe Executor & Evaluator)** · **2.4 WebSocket & Real-time Alert Hub** · **2.5 Technical Debt & Fixes**

Status: ✅ **PHASE 2 COMPLETE** — 2.1–2.5 all implemented & live-verified. Remaining Phase-2 follow-ups are tracked as Phase-3/4 dependencies (frontend `processMessage` update, multi-instance fan-out via Redis Pub/Sub).

---

## 0. Current State (what Phase 2 builds on)

Already in place from Phase 1 — Phase 2 work plugs into these:

| Building block | Location | Notes |
|---|---|---|
| DB schema (migrations) | `backend/migrations/0001_initial_schema.sql` | `alert_points` (UUID v7 PK), `probe_results` (BIGINT identity), `active_alerts` (partial unique index: at most one open alert per point) |
| Models | `backend/src/db/models.rs` | `AlertPoint`, `NewAlertPoint`, `UpdateAlertPoint`, `ProbeResult`, `NewProbeResult`, `ActiveAlert`, `NewActiveAlert`, `ServiceType`, `HttpMethod`, `AlertType` |
| Repository layer | `backend/src/db/repo/{alert_points,probe_results,active_alerts}.rs` | CRUD + `list_recent`, `uptime_percentage`, `insert`/`resolve`/`list_open`/`latest_open` for alerts — **all already compile-time checked with `.sqlx` offline cache** |
| Queue layer | `backend/src/queue/mod.rs` | `ProbeJob` wire contract (`target_id`, `url`, `service_type`, `expected_time_ms`, `timeout_ms`), `enqueue_probe_job` (LPUSH), `dequeue_probe_job` (BRPOP), in-flight gate (`try_acquire_inflight`/`release_inflight`), `ALERTS_PUBSUB_CHANNEL = "sentinel:alerts"`, deadpool pool + health check |
| Config | `backend/src/config.rs` | typed `Config` via env: `APP_HOST`, `APP_PORT`, `APP_CORS_ORIGINS`, `FRONTEND_BUILD_DIR`, `DATABASE_URL`, `REDIS_URL`, `LOG_LEVEL` |
| `AppState` | `backend/src/main.rs` | holds `broadcast_tx: Arc<Sender<Message>>`, `active_alerts: Arc<RwLock<Vec<SentinelAlert>>>` (demo), `db: PgPool`, `redis: RedisPool`, `shutdown_token: CancellationToken` |
| WebSocket handlers | `backend/src/handlers/sentinel_ws.rs` | ✅ task 2.4: snapshot-on-connect + live `WsMessage` events |
| Real-time pipeline | `backend/src/{workers,ws_protocol}` | ✅ tasks 2.2–2.4: scheduler → queue → probe worker → `AlertEvent` broadcast; demo code removed |

**Key gaps to close:** REST API routes, scheduler, probe worker, real alert evaluation, real WS data source, graceful shutdown, tracing IDs. **`reqwest` is not yet a dependency** and must be added.

---

## 1. Dependencies (`backend/Cargo.toml`)

| Crate | Version | Why | Task |
|---|---|---|---|
| `reqwest` | `0.12`, features `["json", "rustls-tls"]`, `default-features = false` | GIS probe HTTP client (task 2.3). Reuse `rustls` already pulled in by sqlx. | 2.3 |
| `validator` (optional) | `0.19`, feature `["derive"]` | declarative payload validation; alternatively hand-rolled `validate()` methods — decide at implementation time | 2.1 |
| `quick-xml` (optional) | `0.37` | `GetCapabilities` XML parsing for WMS/WFS service-level checks | 2.3 |
| `serde` / `serde_json` | already present | wire contracts | — |

New dev-dependencies: `wiremock` or `httpmock` for probe-client tests (feeds task 5.1).

After adding/removing any `query!` macro inputs: re-run `cargo sqlx prepare` (`SQLX_OFFLINE=true` build must stay green).

---

## 2. Task 2.1 — API Service (Alert Points CRUD) ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):** built as planned below, with these deviations:
> - axum 0.8 path syntax is `/{id}` (not `/:id`).
> - **GET/POST only** per decision — update and delete are POST sub-actions
>   (`POST /{id}/update`, `POST /{id}/delete`), not `PUT`/`DELETE`.
> - Nested router at `/api/v1/alert-points` does **not** match a trailing slash — that falls through to the SPA fallback by design (axum `nest` semantics).
> - Create endpoint takes the `NewAlertPoint` model directly (serde defaults); the update endpoint uses an all-optional payload and rejects no-op bodies with 400.
> - Header-name validation lowercases input so `X-Api-Key` is accepted.
> - New repo queries: `alert_points::list(..., Option<ServiceType>, ...)` and `alert_points::count(...)` — `.sqlx` cache regenerated (14 queries).
> - Installed `sqlx-cli 0.8.6` (`~/.cargo/bin`); created `backend/.env` pointing at `postgres-gs`.
> - Live smoke test passed: create / 422-validation / filter+pagination / detail / partial-update / on-demand probe (real 403 captured with latency + snippet) / 404 / delete-cascade.

### 2.1.1 New module layout

```
backend/src/handlers/
├── mod.rs            # add `pub mod alert_points_api;`
└── alert_points_api.rs   # NEW — all REST handlers + router fn
```

### 2.1.2 Routes (mount in `main.rs`)

```rust
let app = Router::new()
    .fallback(static_handler)
    .route("/healthz", get(healthz))              // see 2.5
    .route("/ws", get(ws_handler))                 // removed in 2.4
    .route("/ws/sentinel", get(ws_sentinel_handler))
    .nest("/api/v1/alert-points", alert_points_api::router())
    ...
```

`alert_points_api::router()` returns:

| Method & path | Handler | Repo fn used | Notes |
|---|---|---|---|
| `GET /api/v1/alert-points` | `list_alert_points` | `alert_points::list` + `alert_points::count` | query params: `?enabled=true/false`, `?service_type=WMS`, `?page=1&per_page=50` (map to `limit`/`offset`; `per_page` capped, e.g. max 200) |
| `POST /api/v1/alert-points` | `create_alert_point` | `alert_points::create` | 201 + created `AlertPoint`; 422 on validation failure |
| `GET /api/v1/alert-points/{id}` | `get_alert_point` | `alert_points::get` + `probe_results::list_recent(id, 50)` | returns `{ alert_point, recent_probes }`; 404 when missing |
| `POST /api/v1/alert-points/{id}/update` | `update_alert_point` | `alert_points::update` | partial update semantics; 404 when missing |
| `POST /api/v1/alert-points/{id}/delete` | `delete_alert_point` | `alert_points::delete` | 204; 404 when rows affected = 0. Probe results & alerts cascade (FK `ON DELETE CASCADE`) |
| `POST /api/v1/alert-points/{id}/test` | `test_alert_point` | `alert_points::get` + probe client | runs one probe **synchronously** (with timeout) and returns the would-be `ProbeResult`; does NOT persist, does NOT alert — read-only diagnostic |

> **API style decision:** GET and POST only. State changes are expressed as POST
> sub-actions (`/{id}/update`, `/{id}/delete`, `/{id}/test`) instead of `PUT`/`DELETE` —
> simpler proxies/CORS and no method-based routing edge cases.

### 2.1.3 Validation rules (`validate_new` / `validate_update` on the models, or a new `backend/src/handlers/validation.rs`)

- `url`: absolute `http`/`https` URL (`url::Url::parse` — add `url` crate, already in tree via reqwest), no credentials in URL.
- `check_interval_seconds`: `> 0` (DB CHECK also enforces); sensible floor, e.g. `>= 10`.
- `expected_response_time_ms`: `> 0`.
- `service_type`: deserialization of `ServiceType` already rejects unknown values → maps to 422.
- `http_method`: GET/POST only (already enum-constrained).
- `custom_headers`: must be a JSON object; reject hop-by-hop headers (`Host`, `Content-Length`, `Connection`, ...).
- `auth_config`: if present, must match a known shape (see 2.3.4).
- `UpdateAlertPoint`: at least one field set (`None`-everything → 400).

### 2.1.4 Error handling contract

- Introduce a shared API error type, e.g. `ApiError` in `backend/src/handlers/error.rs` implementing `IntoResponse`:
  - `400 Bad Request`, `404 Not Found`, `422 Unprocessable Entity` (with field-level messages), `500 Internal` (log full error, return opaque message).
- `impl From<sqlx::Error> for ApiError` mapping `RowNotFound` → 404.
- Uniform JSON error body: `{ "error": { "code": "...", "message": "...", "details": ... } }`.

### 2.1.5 Tests (feeds task 5.1)

- `backend/tests/api_alert_points.rs`: spin up the `Router` with a test DB (`axum::serve` on ephemeral port or `tower::ServiceExt::oneshot`), cover happy path + validation failures + 404s.

---

## 3. Task 2.2 — Scheduler Service (Queue Producer) ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):** built per plan with `last_checked_at`
> (Q1-a) in migration `0002_scheduler_state.sql` (partial index on enabled rows).
> - `workers/scheduler.rs`: 1 s–tick loop (default `SCHEDULER_TICK_SECONDS=5`),
>   `list_due` → `SET NX EX` gate → `ProbeJob{timeout_ms = PROBE_TIMEOUT_MS_DEFAULT}` →
>   `LPUSH` → `touch_last_checked`. Gate released on enqueue failure; worker (2.3)
>   releases on completion.
> - `AppState` now carries typed `Config` (scheduler knobs).
> - `.env.example` updated with `SCHEDULER_TICK_SECONDS`, `PROBE_TIMEOUT_MS_DEFAULT`.
> - Live verification: due query → enqueue (pending 0→1) → "no due targets" while
>   interval pending (proves `touch_last_checked`) → gate holds until TTL expiry
>   (40 s) → re-enqueue (pending 2) → SIGINT graceful stop. Queue + DB smoke tests
>   pass against live Valkey/Postgres.

### 3.1.1 New module

```
backend/src/workers/
├── mod.rs             # add `pub mod scheduler;`
└── scheduler.rs       # NEW
```

### 3.1.2 Design

- `pub async fn run_scheduler(app: AppState)` — spawned from `main` next to the existing demo generator (`tokio::spawn`), replaced demo task removed (2.4/2.5).
- Loop with `tokio::time::interval(Duration::from_secs(1))` as the scheduling tick; every tick:
  1. `SELECT ... FROM alert_points WHERE enabled` — via a **new repo fn** `alert_points::list_due(pool, now)`:
     ```sql
     SELECT ... FROM alert_points
     WHERE enabled
       AND now() >= COALESCE(
             -- next due = last probe of this point + interval, else due immediately
             (SELECT timestamp + make_interval(secs => ap.check_interval_seconds)
              FROM probe_results pr WHERE pr.alert_point_id = ap.id
              ORDER BY timestamp DESC LIMIT 1),
             now())
     ```
     (Alternative: track `last_checked_at` column on `alert_points` in migration `0002` — **simpler and cheaper than the correlated subquery; recommended**. See §8 migration.)
  2. For each due point, build a `ProbeJob` (fields already defined in `queue::ProbeJob`; `timeout_ms` = `min(expected * 3, configured)` or a fixed default like 10 s — decide constant in `config.rs`).
  3. **Duplicate in-flight gate**: before `enqueue_probe_job`, `SETNX` a lock key `sentinel:inflight:{target_id}` with `SET NX EX <check_interval_seconds + 30>`; skip enqueue when the key exists. Worker deletes the key when the job is done (see 2.3). This prevents pile-up when a probe is slow or the worker is down.
  4. `queue::enqueue_probe_job(&app.redis, &job)`.
- Error policy: per-target failures are logged and skipped (one bad row must not kill the loop); DB/Redis connection errors log at `error!` and the loop continues after backoff.
- Respects `app.shutdown_token` in the same `tokio::select!` pattern as the current demo generator.

### 3.1.3 Config additions (`src/config.rs`)

| Env var | Default | Purpose |
|---|---|---|
| `SCHEDULER_TICK_SECONDS` | `5` | how often the scheduler polls for due targets |
| `PROBE_TIMEOUT_MS_DEFAULT` | `10000` | fallback timeout when target has none |

---

## 4. Task 2.3 — Worker Service (GIS Probe Executor & Evaluator) ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):**
> - `ProbeJob` extended per Q2-a: `target_name`, `http_method`, `custom_headers`,
>   `auth` (tagged `AuthSpec` enum) — all `#[serde(default)]` for queue compatibility.
> - `probe/gis.rs`: `build_probe_url` (GetCapabilities / `f=pjson` params, case-
>   insensitive param replacement), `check_service_response` (quick-xml root check,
>   ServiceException detection, OAF `links`, ArcGIS `error` key).
> - `probe/evaluate.rs`: `Evaluator` state machine (Down after `PROBE_FAILURE_THRESHOLD`
>   consecutive failures; Degraded on latency > SLA; recovery on first success;
>   `AlertAction::{None, RaiseNew, UpdateOpen, Resolve}`).
> - **Shared evaluator fix found in live testing:** N workers each keeping their own
>   in-memory state caused duplicate `New` raises (unique constraint caught it).
>   The evaluator is now one `Arc<Mutex<..>>` in `AppState`; plus a defensive
>   fallback downgrades a racing `RaiseNew` to an update of the open alert.
> - `active_alerts::update_open` added for `Update` events.
> - `ws_protocol.rs` (`AlertEvent`, `ServiceStatus`, `WsMessage` envelope) — the
>   worker broadcasts serialized `AlertEvent`s; `sentinel_ws` switches to the
>   envelope in task 2.4.
> - Config: `WORKER_CONCURRENCY=2`, `PROBE_FAILURE_THRESHOLD=2` (min 1).
> - Live verification: dead target → 1× `New`/Down broadcast + probe rows persisted;
>   healthy target → probes persisted, no alert; repointing the dead target →
>   `Remove`/Healthy broadcast; zero worker errors after the shared-evaluator fix;
>   stale jobs for deleted targets fail soft (FK violation logged, gate released).

### 4.1.1 New modules

```
backend/src/probe/mod.rs        # NEW — orchestrator: worker loop
backend/src/probe/client.rs     # NEW — reqwest probe execution
backend/src/probe/gis.rs        # NEW — service-type-specific checks
backend/src/probe/evaluate.rs   # NEW — state transition evaluation
backend/src/workers/mod.rs      # add `pub mod probe_worker;` (loop entrypoint)
```

### 4.1.2 Worker loop (`workers/probe_worker.rs`)

- `pub async fn run_worker(app: AppState, worker_id: usize)` — one or more instances spawned at startup (`WORKER_CONCURRENCY`, default 2).
- Loop: `queue::dequeue_probe_job(&app.redis, Duration::from_secs(5))` → on job: execute probe → evaluate → persist → publish → delete in-flight key. `None` (BRPOP timeout) just re-arms the loop. All under `shutdown_token` select.
- Concurrency *within* a worker: sequential is acceptable initially (BRPOP serializes); parallelism via N workers.

### 4.1.3 Probe client (`probe/client.rs`)

- Build one `reqwest::Client` per worker (pooling), `timeout` set per-job from `ProbeJob.timeout_ms`.
- Apply `custom_headers` (from DB — but note: `ProbeJob` currently does **not** carry headers/auth/method. **Decision needed: extend `ProbeJob`** with `http_method`, `custom_headers: HashMap<String,String>`, `auth: Option<AuthKind>` — recommended, keeps worker from re-reading the DB; see §8 Open Question Q2).
- High-resolution timing via `std::time::Instant`:
  - **TTFB**: wrap the response body future — `send()` returns after headers; that instant = TTFB. Read the body, record total duration.
  - Store `response_time_ms` = total transfer duration (TTFB kept in memory / future column — probe_results has one `response_time_ms` column; use total, document it).
- Return an internal `ProbeOutcome { status_code: Option<i32>, response_time_ms: Option<i32>, is_up: bool, error_message: Option<String>, snippet: Option<String>, service_ok: bool }`.

### 4.1.4 GIS-specific checks (`probe/gis.rs`)

| ServiceType | Strategy |
|---|---|
| `WMS` / `WFS` / `WMTS` | GET `?service=<SVC>&request=GetCapabilities` (append/merge params into configured URL); HTTP 200 + parse XML root, verify expected service element / `ServiceException` absence. `quick-xml` (or lenient string match on `<Capability>`/root tag — decide; quick-xml recommended). |
| `OAF` | GET landing page (`/`), expect JSON with `links` array (OGC API — Features conformance). |
| `ArcGIS_REST` | GET `?f=pjson` health ping → expect JSON without `"error"` key (services directory/health). |
| `HTTP` | Plain request as configured (method/headers); success = 2xx/3xx. |

- HTTP-level failure: connection refused/DNS/timeout → `is_up = false`, `error_message` = error chain. 4xx/5xx → `is_up = false`, status recorded.
- `service_ok` false but HTTP 200 (e.g. `ServiceException` XML) → still `is_up = true` (transport fine) but reason "service error" → counts as **Degraded/Down per SLA** — encode this in the evaluator (below).
- Auth: `auth_config` JSONB shapes — `{"type":"basic","username":...,"password":...}` → reqwest basic auth; `{"type":"bearer","token":...}` → `Authorization: Bearer`; `{"type":"header","name":...,"value":...}` → custom header. Never log secret values (mask in tracing).

### 4.1.5 Alert state evaluation (`probe/evaluate.rs`)

State machine per target (current state = open alert in `active_alerts` via `active_alerts::latest_open`, else "Healthy"):

```
probe ok & latency <= expected  → Healthy
probe ok & latency >  expected  → Degraded
probe failed (conn err, 4xx/5xx, service-level error) → Down
```

Transitions and actions:

| From → To | AlertType emitted | DB writes | Publish |
|---|---|---|---|
| Healthy → Down | `New` (reason: unreachable / HTTP status / service error) | `probe_results::insert` + `active_alerts::insert` | WS broadcast |
| Healthy → Degraded | `New` (reason: latency X ms > expected Y ms) | same | WS broadcast |
| Down/Degraded → Down/Degraded (reason or latency changed materially) | `Update` | `probe_results::insert` + `active_alerts::update` (**new repo fn** — update `reason`/`alert_type` on open alert) | WS broadcast |
| Down/Degraded → Healthy | `Remove` | `probe_results::insert` + `active_alerts::resolve` | WS broadcast |
| Healthy → Healthy | none | `probe_results::insert` only | none |

- Debounce/hysteresis (avoid flapping): require N consecutive failing probes (e.g. 2, configurable `PROBE_FAILURE_THRESHOLD`) before declaring Down; 1 success recovers.
- The `Remove` event must carry enough info for the frontend to drop the card: include `alert_point_id` + `alert_type: "Remove"` in the payload.

### 4.1.6 Persistence & fan-out

- Persist every probe via existing `probe_results::insert` (snippet already truncated there).
- Fan-out (task 2.4): after evaluation, publish a JSON `AlertEvent` to **both**:
  1. Redis Pub/Sub `sentinel:alerts` (existing const) — future multi-instance fan-out, and
  2. directly to `app.broadcast_tx` (tokio broadcast) — current single-process WS hub.

---

## 5. Task 2.4 — WebSocket & Real-time Alert Hub ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):**
> - **Migration `0003_alert_status.sql`**: `active_alerts.status` (HEALTHY/DEGRADED/DOWN,
>   default+backfill `DOWN`) so snapshots report the true current state.
> - `ServiceStatus` moved to `db/models.rs` (repo layer lives in the lib crate);
>   `ws_protocol` re-exports it. DB form UPPERCASE, client JSON form snake_case.
> - `repo::active_alerts::list_open_with_point` — JOIN with `alert_points` for the
>   snapshot payload (`name`, `url`, `service_type`, `expected_response_time_ms`).
> - `sentinel_ws.rs`: on connect sends `WsMessage::Snapshot` built from the DB,
>   then forwards live events from the broadcast channel; echo/ping demo logic
>   removed. On `Lagged(n)` the client keeps the connection (live events resume;
>   full re-sync optimization deferred).
> - Worker broadcasts `WsMessage::Alert(event)` now. **Verified wire shapes**
>   (serde internally-tagged enum flattens the event fields):
>
>   ```json
>   {"type":"snapshot","alerts":[{"alert_id":11,"status":"down","alert_type":"New",...}]}
>   {"type":"alert","alert_id":12,"alert_point_id":"…","alert_type":"New","status":"down",...}
>   ```
>
>   (frontend task 3.1 must handle the flattened `alert` fields).
> - **Demo code deleted**: `handlers/websockets.rs` (+ `/ws` route), `schema.rs`,
>   `workers/alert_workers.rs`; `AppState` no longer holds the in-memory demo
>   alert vec — PostgreSQL is the source of truth for snapshots.
> - Live verification (node native WebSocket client): snapshot-on-connect with the
>   open alert (`status:down`), live `Alert` `New`/Down event received on an open
>   connection, `Remove`/Healthy after repointing the target; DB smoke + queue
>   smoke pass; 21 unit tests; clippy/fmt clean.

### 5.1.1 Replace the demo wire schema (`src/schema.rs`)

`SentinelAlert` (`id: String`, `performance`/`expected`/`up`) is demo-era and diverges from the DB model. Replace with a real event contract:

```rust
// src/ws_protocol.rs (new; replaces schema.rs)
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    /// Initial state sync on connect.
    Snapshot { alerts: Vec<AlertEvent> },
    /// Lifecycle event: New / Update / Remove.
    Alert(AlertEvent),
}

#[derive(Serialize, Clone)]
pub struct AlertEvent {
    pub alert_id: i64,
    pub alert_point_id: Uuid,
    pub alert_type: AlertType,        // New | Update | Remove
    pub name: String,                 // join from alert_points
    pub url: String,
    pub service_type: ServiceType,
    pub status: ServiceStatus,        // Healthy | Degraded | Down
    pub reason: String,
    pub response_time_ms: Option<i32>,
    pub expected_response_time_ms: i32,
    pub triggered_at: DateTime<Utc>,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus { Healthy, Degraded, Down }
```

⚠️ **Breaking change for the frontend** — Phase 3 task 3.1 (`processMessage`) must be updated in lockstep. Keep `phase2.md` and the frontend ticket in sync; consider shipping both together.

### 5.1.2 `AppState` changes (`src/main.rs`)

```rust
pub struct AppState {
    broadcast_tx: Arc<Sender<Message>>,      // unchanged
    // REMOVE: active_alerts: Arc<RwLock<Vec<SentinelAlert>>>  (demo state)
    // REMOVE: demo SentinelAlert seed vec in main()
    db: PgPool, redis: RedisPool, shutdown_token: CancellationToken,  // unchanged
}
```

The "full active alert list" no longer lives in memory — it is read from PostgreSQL (`active_alerts::list_open`) on each client connect (source of truth = DB; in-memory mirror is a cache optimization deferred until needed).

### 5.1.3 `sentinel_ws.rs` rework

- On connect (replaces demo `active_alerts.read()` block):
  1. `active_alerts::list_open(&app.db)` → build `Vec<AlertEvent>` (join alert-point metadata; either a new repo fn `list_open_with_point` using a JOIN, or N+1 `alert_points::get` — prefer the JOIN).
  2. Send `WsMessage::Snapshot { alerts }` as the first frame.
  3. Then subscribe to `broadcast_tx` and forward live `Alert` events (existing send-task loop is kept).
- Inbound client messages: keep minimal (echo/ping); define a proper client→server envelope only when the frontend needs it (Phase 3).

### 5.1.4 Redis Pub/Sub bridge (recommended for correctness under multi-process, optional for single node)

- New `backend/src/workers/pubsub_bridge.rs`: task subscribing to `sentinel:alerts`, forwarding every message into `broadcast_tx`.
- Worker publishes to Pub/Sub **only**; bridge fans into the in-process tokio broadcast. (Simpler alternative: worker publishes directly to both — acceptable for Phase 2 single-node, note as tech debt.)

### 5.1.5 Cleanup

- **Delete** `backend/src/handlers/websockets.rs` + its `/ws` route and `mod` entry (task explicitly requires this).
- **Delete** `backend/src/workers/alert_workers.rs` demo generator and its spawn in `main.rs` (superseded by scheduler + probe worker).
- **Delete** `backend/src/schema.rs` demo types (replaced by 5.1.1).
- Update `handlers/mod.rs`, `workers/mod.rs`.

---

## 6. Task 2.5 — Technical Debt & Fixes ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):**
> - ✅ Cross-platform embed path (done in Phase 1).
> - **Graceful shutdown** (reviewed the merged implementation; it was correct):
>   SIGINT/SIGTERM → `CancellationToken.cancel()` → axum drains HTTP and upgraded
>   WS connections (handlers abort their tasks on the token and send `Close`
>   frames). Added in this pass: scheduler/worker `JoinHandle`s are now awaited
>   after the server stops (bounded at 30 s, warn-and-continue), then
>   `db_pool.close().await` + `redis_pool.close()`; token cancel is idempotent
>   so server-stop-for-any-reason also reaches the background tasks.
> - **Request tracing IDs**: `tower-http` `request-id` feature —
>   `SetRequestIdLayer` (server-generated UUID v4, honoring incoming
>   `x-request-id`) → `TraceLayer` with a custom `http_request` span carrying
>   `request_id`/`method`/`uri` → `PropagateRequestIdLayer` echoes the id on the
>   response. Handler logs inherit the span, e.g.:
>   `http_request{request_id=my-trace-42 method=POST uri=/api/v1/alert-points}: alert point created`
> - Uniform API error handling was delivered with task 2.1 (`ApiError`).
> - Live verification: header set/honored/propagated; request_id in handler logs;
>   SIGINT with an open WebSocket → snapshot delivered, close frame `code=1000`,
>   all 3 background tasks joined, pools closed, `Server shut down complete`.
>   (Note: lingering zombie PIDs after runs are a sandbox artifact — PID 1 here
>   does not reap orphans; the processes themselves exited cleanly.)

- [x] Cross-platform embed path (done in Phase 1).
- [ ] **Graceful shutdown hardening** (`main.rs`):
  - `CancellationToken` already exists — extend it to:
    1. cancel scheduler + workers (they exit their loops),
    2. drain WS connections: `broadcast_tx` senders dropped → send tasks see `RecvError::Closed` → close frames already implemented in `sentinel_ws.rs`,
    3. `axum::serve(...).with_graceful_shutdown(token.cancelled_owned())`,
    4. `db_pool.close().await` + `redis_pool.close()` after server exit.
  - Worker loops must finish the in-flight probe before exiting (check token between jobs, not mid-probe).
- [ ] **Request tracing IDs**:
  - Add `tower-http` `RequestIdLayer` + `PropagateRequestIdLayer` (already have `tower-http` 0.6) or a small custom middleware; log `request_id` in `TraceLayer` spans (`make_span_with` including the id header).
  - Structured logging: switch `tracing_subscriber::fmt` to JSON in non-dev (`LOG_FORMAT=json`, default `pretty`) — optional, cheap.
- [ ] `.env.example` additions: `SCHEDULER_TICK_SECONDS`, `WORKER_CONCURRENCY`, `PROBE_TIMEOUT_MS_DEFAULT`, `PROBE_FAILURE_THRESHOLD`, `APP_CORS_ORIGINS`.

---

## 7. File-by-file change summary

| File | Action |
|---|---|
| `Cargo.toml` | add `reqwest`, (opt) `validator`, `quick-xml`, `url`; dev-dep `wiremock` |
| `migrations/0002_scheduler_state.sql` | ✅ DONE — `alert_points.last_checked_at TIMESTAMPTZ` + partial index on enabled rows |
| `src/main.rs` | mount `/api/v1/alert-points` nest; spawn scheduler + N probe workers + pubsub bridge; remove demo worker spawn; graceful shutdown wiring |
| `src/lib.rs` | add `pub mod ws_protocol; pub mod probe;` |
| `src/config.rs` | ✅ DONE — `SCHEDULER_TICK_SECONDS`, `PROBE_TIMEOUT_MS_DEFAULT` (+ `.env.example`) |
| `src/db/models.rs` | add `last_checked_at` to `AlertPoint`; add `ServiceStatus` enum; (opt) `AuthConfig` typed deserializer; add `ProbeJob`-support types if ProbeJob is extended |
| `src/db/repo/alert_points.rs` | ✅ DONE — `list_due(pool)` + `touch_last_checked(id)`; all SELECTs include `last_checked_at` |
| `src/db/repo/active_alerts.rs` | add `update_reason(point_id, reason)` for `Update` events; add `list_open_with_point` JOIN query |
| `src/queue/mod.rs` | (opt) extend `ProbeJob` with method/headers/auth; add `publish_alert_event()` + `subscribe_alerts()` Pub/Sub helpers |
| `src/handlers/mod.rs` | add `alert_points_api`, `error` modules |
| `src/handlers/alert_points_api.rs` | **NEW** — 6 REST handlers + router + pagination/filter params |
| `src/handlers/error.rs` | **NEW** — `ApiError` + `IntoResponse` |
| `src/handlers/sentinel_ws.rs` | ✅ DONE — snapshot-on-connect from DB, `WsMessage` protocol, demo reads/echo removed |
| `src/handlers/websockets.rs` | ✅ DELETED |
| `src/handlers/mod.rs` | ✅ DONE — `websockets` removed |
| `src/schema.rs` | ✅ DELETED (replaced by `ws_protocol.rs`) |
| `src/ws_protocol.rs` | ✅ DONE — `WsMessage::{Snapshot, Alert}`, `AlertEvent`; `ServiceStatus` lives in `db/models.rs` and is re-exported |
| `src/probe/mod.rs`, `src/probe/client.rs`, `src/probe/gis.rs`, `src/probe/evaluate.rs` | ✅ DONE — probe execution + evaluation engine |
| `src/workers/mod.rs` | ✅ DONE — `scheduler`, `probe_worker` |
| `src/workers/scheduler.rs` | ✅ DONE — due-target polling + in-flight gate + enqueue; spawned from `main` |
| `src/workers/probe_worker.rs` | ✅ DONE — BRPOP loop → probe → evaluate → persist → broadcast (in `WsMessage::Alert` envelope) → gate release |
| `src/workers/pubsub_bridge.rs` | deferred (multi-instance upgrade path; single-process tokio broadcast suffices for Phase 2) |
| `src/workers/alert_workers.rs` | ✅ DELETED |
| `tests/queue_smoke.rs` | keep; add `tests/api_alert_points.rs`, `tests/probe_eval.rs` (unit) |

---

## 8. Open Questions (need decisions before/during implementation)

| # | Question | Options | Recommendation |
|---|---|---|---|
| Q1 | Scheduler "due" tracking | a) `last_checked_at` column + migration 0002 (recommended), b) correlated subquery on `probe_results` per tick | **a** — one column, trivially indexable |
| Q2 | `ProbeJob` payload | a) extend with `http_method`, `custom_headers`, `auth` (recommended — worker stays DB-free), b) worker re-reads `alert_points` row by `target_id` before probing | **a**; field additions are backward-compatible while queue is empty |
| Q3 | WS fan-out topology | a) worker → Redis Pub/Sub → bridge → tokio broadcast (multi-node ready), b) worker → tokio broadcast directly (single-node, simpler) | **b for Phase 2**, leave Pub/Sub const + bridge as the documented upgrade path |
| Q4 | Concurrent failing-probe threshold | flap protection N=1 (alert immediately) vs N=2–3 (stable but delayed) | **N=2** via `PROBE_FAILURE_THRESHOLD`, `1` disables |
| Q5 | Validation approach | a) `validator` crate derive, b) hand-written `validate()` returning `Vec<(field, msg)>` | **b** — fewer deps, validation is bespoke (URL + service-type pairing) |
| Q6 | On-demand `/test` endpoint semantics | a) run probe inline synchronously and return result (simplest, bounded by timeout), b) enqueue high-priority job + poll result | **a** |

---

## 9. Verification plan (mirrors phase1.md style)

1. `cargo check` / `cargo clippy` / `cargo fmt --check` clean; `cargo sqlx prepare` re-run after any new query macros.
2. `cargo test` — new unit tests for evaluation state machine (no network) + `wiremock`-based probe tests + API integration tests against test DB.
3. Manual end-to-end against shared instance (`postgres-gs`, Valkey container from task 1.4):
   - insert an `alert_point` via `POST /api/v1/alert-points` pointing at a deliberately slow/dead URL,
   - observe scheduler enqueue (`LLEN sentinel:probe_jobs`), worker probe, `probe_results` row, `active_alerts` row,
   - connect to `wss://.../ws/sentinel` and verify `Snapshot` + live `Alert` events for New/Update/Remove,
   - `Ctrl-C` → verify graceful drain in logs.
