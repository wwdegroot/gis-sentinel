# GIS Sentinel - Tasks & Roadmap

This document outlines the roadmap and missing components for **GIS Sentinel**, based on the architecture described in [README.md](./README.md) and the current state of `backend/` and `frontend/`.

> **Progress:** Phase 1 ✅ (`phase1.md`) · Phase 2 ✅ (`phase2.md`) · Phase 3 ✅ (`phase3.md`) · Phase 4+ pending

---

## 📊 Current State vs Target Architecture Gap Analysis

| Component | Target Architecture (`README.md`) | Current Status |
|---|---|---|
| **Database** | PostgreSQL for storing monitoring targets, configs, and history | ❌ Missing (No DB connection, schema, or migrations) |
| **Queue / Broker** | Valkey / Redis for alert jobs and worker coordination | ❌ Missing (No Redis/Valkey integration) |
| **Backend API Service** | REST API for CRUD operations on Alert Points | ❌ Missing (Only mock in-memory alerts exist) |
| **Backend Scheduler** | Periodically enqueues check jobs into Valkey/Redis | ❌ Missing |
| **Backend Worker** | Consumes queue, probes GIS endpoints (WMS/WFS/REST), evaluates thresholds | ❌ Missing |
| **Backend Web Server** | Axum server with WebSockets and embedded frontend | ⚠️ Partial (Basic WebSocket echo & mock alert streaming; Windows-specific embed path) |
| **Frontend Dashboard** | Real-time monitoring dashboard with live status cards and metrics | ⚠️ Partial (Minimal raw key-value display prototype) |
| **Frontend CRUD UI** | Management interface to configure GIS endpoints, thresholds, intervals | ❌ Missing |
| **DevOps / Infra** | Docker compose, CI/CD, production builds | ❌ Missing |

---

## 🚀 Phase 1: Infrastructure & Data Architecture

- [ ] **1.1 PostgreSQL Database Integration**
  - [x] Choose and configure an async query engine `sqlx` with Postgres driver. (sqlx 0.8, compile-time checked `query!` macros + committed `.sqlx` offline cache; see `phase1.md`)
  - [x] Implement database migrations (`sqlx-cli` or embedded migrations). (embedded `sqlx::migrate!()`, applied at startup; `sqlx-cli` 0.8.6 for `prepare`)
  - [x] Design schema for:
    - **`alert_points`**: id, name, url, service_type (`WMS`, `WFS`, `WMTS`, `OAF`, `ArcGIS_REST`, `HTTP`), check_interval_seconds, expected_response_time_ms, http_method, custom_headers, auth_config, enabled, created_at, updated_at. (`migrations/0001_initial_schema.sql`)
    - **`alert_logs` / `probe_results`**: id, alert_point_id, timestamp, response_time_ms, status_code, is_up, error_message, raw_response_snippet.
    - **`active_alerts`**: id, alert_point_id, alert_type (`New`, `Update`, `Remove`), reason, triggered_at, resolved_at.

- [x] **1.2 Valkey / Redis Queue Integration**
  - [x] Add Valkey client dependency. (`redis` 0.27 + `deadpool-redis` 0.18 pooled client — Valkey is Redis-compatible; see `phase1.md`)
  - [x] Define queue data structures (job payload schema with Serde: `target_id`, `url`, `service_type`, `expected_time_ms`, `timeout_ms`). (`src/queue/mod.rs` → `ProbeJob`, list key `sentinel:probe_jobs`)
  - [x] Implement Redis connection pooling and health checks. (`queue::connect()` pool max 10, `queue::health_check()` PING; pool in `AppState`)

- [x] **1.3 Environment & Configuration Management**
  - [x] Create structured configuration module using `config` or `dotenvy` / `envy`. (`src/config.rs`, typed `Config` via `envy`, fail-fast on missing `DATABASE_URL`)
  - [x] Define `.env.example` with: `DATABASE_URL`, `REDIS_URL`, `SERVER_HOST`, `SERVER_PORT`, `LOG_LEVEL`.
  - [x] Remove hardcoded host/port (`127.0.0.1:3000`) in backend and frontend. (backend binds via `SERVER_HOST`/`SERVER_PORT`; frontend uses same-origin `location.host` + `vite.config.ts` dev proxy to backend)

- [x] **1.4 Local Development Environment**
  - [x] Update `docker/docker-compose.yml` defining:
    - Valkey / Redis container (`valkey-gs`, valkey 8.1.3-alpine, AOF persistence + healthcheck)
    - pgAdmin (port 5050) / Redis Commander (port 8081) for dev inspection.

---

## ⚙️ Phase 2: Backend Services Implementation

- [x] **2.1 API Service (Alert Points CRUD)** ✅ implemented (see `phase2.md` §2)
  - [x] Create REST routes under `/api/v1/alert-points` (GET/POST only; state changes as POST sub-actions):
    - [x] `GET /api/v1/alert-points` - List all configured monitoring points (with filtering & pagination).
    - [x] `POST /api/v1/alert-points` - Create a new monitoring point.
    - [x] `GET /api/v1/alert-points/:id` - Fetch single monitoring point details and recent probe history.
    - [x] `POST /api/v1/alert-points/:id/update` - Update monitoring point settings (partial).
    - [x] `POST /api/v1/alert-points/:id/delete` - Delete a monitoring point.
    - [x] `POST /api/v1/alert-points/:id/test` - Trigger an on-demand probe check. (HTTP-level transport probe; GIS-specific checks land with 2.3)
  - [x] Implement request payload validation (valid URLs, positive intervals/thresholds, recognized GIS service types).
  - Note: axum 0.8 nested routes match `/api/v1/alert-points` but **not** `/api/v1/alert-points/` (trailing slash falls through to the SPA fallback).

- [x] **2.2 Scheduler Service (Queue Producer)** ✅ implemented (see `phase2.md` §3)
  - [x] Implement background scheduler loop (using `tokio::time::interval` or `tokio-cron-scheduler`). (`workers/scheduler.rs`, plain `interval` + `CancellationToken`)
  - [x] Periodically query enabled `alert_points` from database based on their `check_interval_seconds`. (`alert_points::list_due` via `last_checked_at`, migration `0002_scheduler_state.sql`)
  - [x] Push probe jobs into the Valkey/Redis queue avoiding duplicate in-flight checks. (`SET NX EX` gate `sentinel:inflight:{target_id}` in `queue::try_acquire_inflight`; TTL = interval + 30 s, worker releases explicitly in 2.3)

- [x] **2.3 Worker Service (GIS Probe Executor & Evaluator)** ✅ implemented (see `phase2.md` §4)
  - [x] Implement worker loop pulling jobs from the Redis queue. (`workers/probe_worker.rs`, BRPOP + `CancellationToken` + `WORKER_CONCURRENCY` instances sharing one `Evaluator`)
  - [x] Build GIS probe client using `reqwest`:
    - [x] Support HTTP GET/POST with customizable headers/timeout. (`ProbeJob` carries method/headers/auth — Q2-a)
    - [x] GIS-specific checks (e.g. `GetCapabilities` XML parsing for WMS/WFS, health ping for ArcGIS Server). (`probe/gis.rs` via quick-xml)
    - [x] Measure high-resolution latency (TTFB and total transfer duration). (`Instant`-based; total persisted, TTFB logged at debug)
  - [x] Implement alert state evaluation logic:
    - [x] Check if service is unreachable or returns HTTP 4xx/5xx errors. (transport + GIS service-level checks)
    - [x] Compare actual latency against `expected_response_time_ms`. (Degraded status)
    - [x] Detect state transitions (`Up` -> `Down`/`Degraded`, `Down` -> `Recovered`). (with `PROBE_FAILURE_THRESHOLD` hysteresis)
  - [x] Persist probe metrics to PostgreSQL. (every probe via `probe_results::insert`)
  - [x] Push state transition alerts to Redis Pub/Sub or backend broadcast channel. (tokio broadcast `AlertEvent` per Q3-b; Pub/Sub is the 2.4 upgrade path)

- [x] **2.4 WebSocket & Real-time Alert Hub** ✅ implemented (see `phase2.md` §5)
  - [x] Connect WebSocket broadcaster to real-time worker events (via Redis PubSub or Tokio broadcast). (tokio broadcast; worker publishes `WsMessage::Alert` JSON)
  - [x] Implement initial state synchronization when frontend clients connect (send full active alert list). (`active_alerts::list_open_with_point` JOIN → `Snapshot`, status persisted via migration `0003`)
  - [x] Handle incremental alert lifecycle events: `New` / `Update` / `Remove`. (evaluation → DB action → broadcast, task 2.3)
  - [x] Clean up legacy demo code in `handlers/websockets.rs` and consolidate into `handlers/sentinel_ws.rs`. (deleted `websockets.rs`, `schema.rs`, `workers/alert_workers.rs`; `/ws` route removed)

- [ ] **2.5 Backend Technical Debt & Fixes** ✅ implemented (see `phase2.md` §6)
  - [x] Fix cross-platform static asset embed path in `backend/src/handlers/generic.rs` (change `r"..\frontend\build\"` to `"../frontend/build"`). (done early as part of 1.1 — blocked all Linux builds)
  - [x] Add graceful shutdown handling for Tokio tasks, DB pools, and server listeners. (signal → `CancellationToken` → HTTP drain incl. WS close frames → background tasks joined (30 s bound) → sqlx/deadpool pools closed)
  - [x] Improve error handling and structured logging with request tracing IDs. (`x-request-id` set/honored/propagated + stamped into `http_request` span; uniform `ApiError` JSON from 2.1)

---

## 🎨 Phase 3: Frontend Development & UI/UX

- [x] **3.1 WebSocket Client Improvements (`sentinelSocket.svelte.ts`)** ✅ implemented (see `phase3.md` §1)
  - [x] Fix typo: rename `adress` to `address`.
  - [x] Implement `Update` and `Remove` handling in `processMessage` (update existing alert by id, remove resolved alert). (keyed by `alert_id`; snapshot fully replaces state; New is idempotent)
  - [x] Add auto-reconnect with exponential backoff and connection state indicators (Connected, Reconnecting, Disconnected). (500 ms → 30 s cap + jitter, reset on open; `ConnectionState` + `reconnecting` getter)
  - [x] Implement dynamic WebSocket URL resolution using `window.location.host` and `ws:`/`wss:` protocols. (`resolveWebSocketUrl()`)

- [x] **3.2 Application Layout & Navigation** ✅ implemented (see `phase3.md` §2)
  - [x] Replace temporary demo root `routes/+page.svelte` with a proper landing dashboard. (echo playground deleted; dashboard lands with 3.3)
  - [x] Add a global navigation bar in `routes/+layout.svelte`:
    - [x] **Dashboard** (`/` or `/`) - Real-time alerts and service status overview. (`/sentinel` client-side-redirects to `/`)
    - [x] **Services / Alert Points** (`/services`) - List and manage GIS endpoints. (stub page; UI in 3.4)
    - [x] **History / Logs** (`/history`) - Historical probe records and uptime metrics. (stub page; UI in 3.5)
    - [x] **Settings** (`/settings`) - Global thresholds and notification settings. (stub with working theme control)
  - [x] Add dark/light mode and Tailwind theme styling. (Tailwind 4 `@custom-variant dark`, `@theme inline` runtime tokens, pre-paint script, persisted toggle, dark default)

- [x] **3.3 Live Sentinel Monitoring Dashboard (`/sentinel`)** ✅ implemented (see `phase3.md` §3)
  - [x] Redesign alert cards with GIS service metadata:
    - [x] Service name, endpoint URL, service type badge (WMS, WFS, REST).
    - [x] Status badge: `Healthy`, `Degraded (Slow)`, `Down (Unreachable)`. (`StatusBadge`)
    - [x] Latency bar / meter (current vs expected SLA). (`LatencyMeter`, 0–2× SLA track)
    - [x] Failure reason / HTTP status / error snippet. (reason text on card)
    - [x] Duration of current outage / alert timestamp. (ticking `formatDuration` from `triggered_at`)
  - [x] Summary statistics header (Total monitored, Total Healthy, Active Incidents, Average Latency). (`SummaryStats`, total via REST, rest derived from socket state)
  - [x] Filters by service type, status, and search query. (client-side `$derived` filtering + sorting by severity/recency)

- [x] **3.4 Alert Points Configuration Interface (`/services`)** ✅ implemented (see `phase3.md` §4)
  - [x] Data table showing all monitored GIS services with quick toggles (enable/disable). (`ServiceTable`, optimistic toggle with rollback)
  - [x] Modal/Form for adding and editing alert points:
    - [x] Target URL, Name, GIS Service Type.
    - [x] Expected SLA Latency (ms), Check Interval (seconds), Timeout (seconds). (interval + SLA; probe timeout is a backend default — field planned with per-target timeout column)
    - [x] Custom headers, HTTP Basic Auth / API token. (key/value rows; basic/bearer/header auth kinds)
  - [x] "Test Now" button with immediate probe result feedback. (`TestResultPanel`)
  - [x] Delete confirmation modal. (`DeleteConfirm`)

- [x] **3.5 Historical Analytics & Service Detail View** ✅ implemented (see `phase3.md` §5)
  - [x] Time-series latency charts for monitored endpoints. (`LatencyChart`, hand-rolled SVG — zero deps; SLA dashed line, red markers for failed probes, window-spanning x axis)
  - [x] Uptime percentage over 24h / 7d / 30d periods. (`UptimeStats`, three parallel history fetches)
  - [x] Incident log history table with export to CSV/JSON. (`IncidentTable`, client-side blob download)

---

## 📦 Phase 4: DevOps, Build & Deployment

- [ ] **4.1 Production Multi-Stage Dockerfile**
  - [ ] Stage 1: Build frontend with Node.js & pnpm (`pnpm build`).
  - [ ] Stage 2: Build Rust backend with `cargo build --release` (embedding frontend static build).
  - [ ] Stage 3: Minimal runtime image (`debian-slim` or `alpine`) containing the single unified binary.

- [ ] **4.2 Continuous Integration & Testing (CI/CD)**
  - [ ] GitHub Actions workflow for:
    - Backend: `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt --check`.
    - Frontend: `pnpm svelte-check`, `pnpm lint`, `pnpm test`, `pnpm build`.

- [ ] **4.3 Production Deployment & Health Checks**
  - [ ] Add `/healthz` and `/livez` HTTP endpoints for container orchestration / Kubernetes readiness probes.
  - [ ] Add documentation for production deployment and environment configuration.

---

## 🧪 Phase 5: Testing & Quality Assurance

- [ ] **5.1 Backend Tests**
  - [ ] Unit tests for GIS probe parsing and SLA calculation.
  - [ ] Unit tests for alert state transitions (`New`, `Update`, `Remove`).
  - [ ] Integration tests for REST CRUD API endpoints using Axum test utilities.
  - [ ] Mock tests for Redis queue producer/consumer flows.

- [ ] **5.2 Frontend Tests**
  - [ ] Unit tests for `SentinelSocket` state machine (connection, reconnection, message routing).
  - [ ] Component tests for Alert Card, Status Badges, and Form validation.
  - [ ] End-to-end tests (Playwright) covering service creation and real-time alert rendering.

---

## 📌 Suggested Immediate Next Steps

1. ✅ **Fix cross-platform embed path** in `backend/src/handlers/generic.rs` to allow compiling on Linux/macOS. (done as part of 1.1)
2. ✅ **Add `docker-compose.yml`** with Postgres and Valkey/Redis for local service dependencies. (done as part of 1.4)
3. ✅ **Implement Database & Schema** for `alert_points` and connect backend via `sqlx`. (done as part of 1.1, incl. repository layer + offline query cache)
4. ✅ **Implement REST CRUD Endpoints** in backend (done as part of 2.1; the `/services` frontend page is task 3.4) and build the `/services` frontend management page.
5. ✅ **Implement Scheduler & Worker** loop to turn GIS Sentinel from mock data into a functioning monitoring engine. (done as part of 2.2–2.4, incl. real-time WebSocket hub)

→ Next up: **Phase 3 (Frontend)** — starting with task 3.1, since the backend WS wire contract changed in Phase 2.
