# Phase 1.1 — PostgreSQL Database Integration (Plan)

Scope: `tasks.md` → Phase 1 → **1.1 PostgreSQL Database Integration**

Status: ✅ **IMPLEMENTED & VERIFIED** — all decisions confirmed; see section 8 for the implementation report.

---

## 0. Key Decisions (confirmed)

| # | Question | Options | Decision |
|---|----------|---------|--------------------|
| Q1 | Query checking style | a) `sqlx::query!` compile-time checked macros (requires `DATABASE_URL` at build time or `sqlx prepare` offline mode), b) runtime unchecked queries | **a) compile-time checked with offline `.sqlx` cache (committed)** |
| Q2 | Primary key strategy for `alert_points` | a) `BIGINT GENERATED ALWAYS AS IDENTITY`, b) `UUID v7` | **b) UUID v7** (app-generated via `Uuid::now_v7()`); `probe_results`/`active_alerts` use `BIGINT` identity |
| Q3 | `service_type` representation | a) Postgres `ENUM` type, b) `TEXT` + `CHECK` constraint, c) plain `TEXT` validated only in Rust | **b) TEXT + CHECK** |
| Q4 | `active_alerts` cardinality | Can one `alert_point` have multiple simultaneously active alerts, or at most one? | **at most one** — partial unique index on `alert_point_id WHERE resolved_at IS NULL` |
| Q5 | Local dev Postgres | Docker not available in this environment. How do you want to run/test Postgres here? (a) you provide a `DATABASE_URL` to an existing instance, b) skip live verification locally and rely on `cargo build` + later docker-compose (task 1.4), c) install/run Postgres natively | **build-only + local verification** via portable PostgreSQL 16.15 binaries (`postgresql-binaries`, no root/docker); compose file deferred to task 1.4 |
| Q6 | Wire into app now? | Only build the DB layer, or also inject `PgPool` into `AppState` (unused by routes yet)? | **yes** — `PgPool` in `AppState`, migrations applied at startup |
| Q7 | Migration tooling | a) `sqlx-cli` (dev dependency / CI step), b) embedded `sqlx::migrate!()` run at startup | **b) embedded `sqlx::migrate!()`** applied at startup; `sqlx-cli` (matching 0.8.6) only for `prepare` |
| Q8 | Time & timestamp types | a) `chrono` crate, b) `time` crate, c) `sqlx` built-ins | **a) chrono** (`DateTime<Utc>`), timestamps stored as `TIMESTAMPTZ` |

---

## 1. Dependencies (`backend/Cargo.toml`)

- `sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "uuid", "chrono", "migrate"] }`
- `uuid = { version = "1", features = ["v7", "serde"] }`
- `chrono = { version = "0.4", features = ["serde"] }`
- `dotenvy = "0.15"` — loads `.env` (`DATABASE_URL`, `SQLX_OFFLINE`) at startup (full config module is task 1.3)

## 2. Schema / Migrations (`backend/migrations/0001_initial_schema.sql`)

```sql
CREATE TABLE alert_points (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),  -- app sets v7 on insert
    name          TEXT NOT NULL,
    url           TEXT NOT NULL,
    service_type  TEXT NOT NULL CHECK (service_type IN ('WMS','WFS','WMTS','OAF','ArcGIS_REST','HTTP')),
    check_interval_seconds       INT  NOT NULL CHECK (check_interval_seconds > 0),
    expected_response_time_ms    INT  NOT NULL CHECK (expected_response_time_ms > 0),
    http_method   TEXT NOT NULL DEFAULT 'GET' CHECK (http_method IN ('GET','POST')),
    custom_headers JSONB NOT NULL DEFAULT '{}'::jsonb,
    auth_config   JSONB,                                        -- NULL = no auth
    enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_alert_points_enabled ON alert_points (enabled) WHERE enabled;

CREATE TABLE probe_results (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    alert_point_id UUID NOT NULL REFERENCES alert_points(id) ON DELETE CASCADE,
    timestamp     TIMESTAMPTZ NOT NULL DEFAULT now(),
    response_time_ms INT,
    status_code   INT,
    is_up         BOOLEAN NOT NULL,
    error_message TEXT,
    raw_response_snippet TEXT
);
CREATE INDEX idx_probe_results_point_time ON probe_results (alert_point_id, timestamp DESC);

CREATE TABLE active_alerts (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    alert_point_id UUID NOT NULL REFERENCES alert_points(id) ON DELETE CASCADE,
    alert_type     TEXT NOT NULL CHECK (alert_type IN ('New','Update','Remove')),
    reason         TEXT NOT NULL,
    triggered_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at    TIMESTAMPTZ
);
-- at most one active alert per point (see Q4)
CREATE UNIQUE INDEX uq_active_alerts_point ON active_alerts (alert_point_id) WHERE resolved_at IS NULL;
```

- `updated_at` kept consistent via application code (simplest) or a `BEFORE UPDATE` trigger ❓ (proposal: trigger in SQL for correctness).
- `raw_response_snippet` limited to ~2 KB at insert time to bound row size.

## 3. Backend Code Structure

```
backend/src/
├── db/
│   ├── mod.rs        -- connect(), run_migrations(), DbError
│   ├── models.rs     -- AlertPoint, ProbeResult, ActiveAlert (+ FromRow, sqlx::Type enums, serde)
│   └── repo/
│       ├── mod.rs
│       ├── alert_points.rs   -- CRUD queries (query! macros)
│       ├── probe_results.rs  -- insert, list-recent, aggregate helpers
│       └── active_alerts.rs  -- trigger/resolve/latest-open queries
└── state / main.rs   -- PgPool added to AppState (see Q6)
```

- Pool config: `max_connections` (default 5), `acquire_timeout` 5 s, `min_connections` 1.
- Startup flow in `main.rs`: load `DATABASE_URL` → `connect()` → run embedded migrations (via `sqlx::migrate!()`) → log schema version → continue serving; abort startup with a clear error if DB unreachable.
- No repository is called by routes yet — this step is purely infrastructure for Phase 2.

## 4. Config

- `DATABASE_URL` read from environment, `.env` loaded via `dotenvy`. Full config module stays in task 1.3.
- `SQLX_OFFLINE=true` (from `.env`) forces compile-time query checks to use the committed `.sqlx/` cache, so builds never require a live DB.

## 5. Verification Steps

1. `cargo build` compiles with compile-time checked queries (offline mode via `.sqlx/` cache, or live DB — depends on Q1/Q5).
2. `cargo clippy -- -D warnings` clean, `cargo fmt`.
3. If a live Postgres is available (Q5): migrations apply cleanly on empty DB, idempotent on second run.
4. Optional smoke test: small `#[cfg(test)]` integration test or a `sqlx-cli`-less one-off binary that inserts/reads an `alert_point`.

## 6. Explicit Non-Goals (deferred)

- Redis/Valkey (1.2), full config module (1.3), `docker-compose.yml` (1.4) — but note task 1.4 is the natural follow-up to test the DB locally.
- REST CRUD endpoints (2.1) — repository layer only, no routes.
- Data retention/cleanup job for `probe_results` (future task, index already supports it).

## 7. Dev Workflow for Schema/Query Changes

1. Point `DATABASE_URL` at a live Postgres and unset `SQLX_OFFLINE` (or set `false`).
2. Apply the schema to that DB (start the app once — embedded migrator runs, or `sqlx migrate run`).
3. Edit queries / add migrations; refresh the offline cache: `cargo sqlx prepare -- --all-targets` (run from `backend/`, commit `.sqlx/`).
4. Restore `SQLX_OFFLINE=true` for normal builds.

## 8. Implementation Report (what was actually built)

**Files added**
- `backend/migrations/0001_initial_schema.sql` — schema as designed above, plus a `BEFORE UPDATE` trigger keeping `alert_points.updated_at` current.
- `backend/src/db/mod.rs` — `connect()` (pool: max 5, min 1, acquire timeout 5 s), `run_migrations()` via embedded `sqlx::migrate!()`.
- `backend/src/db/models.rs` — `AlertPoint`, `NewAlertPoint`, `UpdateAlertPoint`, `ProbeResult`, `NewProbeResult`, `ActiveAlert`, `NewActiveAlert` + `ServiceType`, `HttpMethod`, `AlertType` enums (`sqlx::Type` text encoding matching DB CHECK constraints, serde-compatible).
- `backend/src/db/repo/alert_points.rs` — create/list (filter + pagination)/get/update (COALESCE partial update; auth_config "unchanged vs set-to-NULL" distinguishable)/delete.
- `backend/src/db/repo/probe_results.rs` — insert (raw snippet truncated to 2000 chars), list_recent, uptime_percentage.
- `backend/src/db/repo/active_alerts.rs` — insert, resolve (marks open alerts resolved), list_open (WS initial sync source), latest_open.
- `backend/.env.example`, committed `.sqlx/` offline query cache.
- `backend/src/main.rs` — `db_smoke::repository_layer_round_trip` test, `#[ignore]`-gated, run via `cargo test -- --ignored`.

**Files modified**
- `backend/Cargo.toml` — added `sqlx` (postgres/tokio/rustls/uuid/chrono/migrate), `uuid` (v7), `chrono`, `dotenvy`.
- `backend/src/main.rs` — dotenvy load, `DATABASE_URL` → `db::connect()` → migrations at startup (aborts with clear error on failure); `PgPool` added to `AppState`.
- `backend/src/handlers/generic.rs` — fixed Windows-only rust-embed path `r"..\frontend\build\"` → `"../frontend/build"` (required for any Linux/macOS build; corresponds to task 2.5 item 1).
- `backend/src/handlers/websockets.rs`, `backend/src/handlers/sentinel_ws.rs`, `backend/src/main.rs` — trivial clippy lint fixes (unused import/variable, dead-code allows) so `cargo clippy --all-targets -- -D warnings` passes (needed by CI task 4.2).

**Verification performed**
- `cargo build` succeeds fully offline (compile-time checked queries against committed `.sqlx` cache).
- `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt -- --check` clean.
- Portable PostgreSQL 16.15 (no docker/root) started locally; app startup applied migration 0001 to empty DB; server boots and serves `HTTP 200` on `/`.
- `cargo test -- --ignored` passes: full repository round trip (create → read → update → probe insert/truncation → uptime aggregation → alert trigger/unique-violation/resolve → cascade delete).
