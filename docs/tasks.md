Backend Code Review: GIS Sentinel

## Overview

The backend is an Axum-based Rust application serving two WebSocket endpoints (`/ws` and `/ws/sentinel`) and embedding a SvelteKit frontend as static assets. It's clearly a proof of concept — functional but missing many production-ready foundations.

---

## Critical Issues (Must Fix)

### 1. Remove Dead / Demo Code
- **File:** `src/handlers/websockets.rs` — This entire file is copied from the Axum example. The `/ws` endpoint sends hardcoded "Hi X times!" and "Server message X..." messages. It doesn't use `AppState` at all (unused parameters trigger warnings). Decide: either integrate it meaningfully or delete it entirely.

### 2. FIXED: Hardcoded Bind Address & Crashing `.unwrap()` Calls
- **File:** `src/main.rs:L63-L64` — `tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap()` will crash if the port is in use or binding fails. Same for the tracing subscriber init on L35.
- **Fix:** Use environment variables or a config file for the bind address/port, and propagate errors instead of unwrapping. 

### 3. FIXED: No Configuration Management
- Port, host, CORS origins, alert thresholds — everything is hardcoded.
- **Fix:** Introduce a `Config` struct loaded from `.env` (using `dotenvy`) or a TOML/JSON config file with sensible defaults.

### 4. Fragile Frontend Path Embedding
- **File:** `src/handlers/generic.rs:L12` — `#[folder = r"..\frontend\build\"]` uses a Windows-specific relative path. This will break on Linux/macians and in CI.
- **Fix:** Use an environment variable or build script to resolve the frontend output directory, e.g.:
  ```rust
  const FRONTEND_BUILD: &str = env!("FRONTEND_BUILD_DIR");
  #[folder = "$FRONTEND_BUILD_DIR"]
  struct Assets;
  ```

### 5. FIXED: No Graceful Shutdown
- **File:** `src/main.rs` — When the process receives SIGINT/SIGTERM, connections are dropped abruptly.
- **Fix:** Use `tokio::signal` to catch shutdown signals and call `server.with_graceful_shutdown(...)` to drain active WebSocket connections.

---

## High Priority (Should Fix)

### 6. No Authentication / Authorization
- Both WebSocket endpoints are publicly accessible — anyone can connect, send messages, and receive alerts.
- **Fix:** Add at minimum an API key or JWT-based auth middleware for the sentinel endpoint. Consider per-client permissions if multiple teams will use this.

### 7. Broadcast Channel Drops Messages
- **File:** `src/main.rs:L32` — `broadcast::channel(32)` only holds 32 messages. When it fills up, older messages are silently dropped (L59 in `sentinel_ws.rs` just logs and continues).
- **Fix:** Either increase capacity significantly, implement a queue with backpressure, or use a per-client message buffer so late-connecting clients don't miss state updates.

### 8. No Alert Persistence Layer
- **File:** `src/main.rs:L37-L56` — Alerts are hardcoded as static data in `main()`. There's no database, no way to add/remove alerts at runtime (beyond echoing messages).
- **Fix:** Introduce a persistence layer:
  - Start with an in-memory store wrapped behind a trait (for testability)
  - Progress to SQLite (`sqlx` or `rusqlite`) for durability
  - Add CRUD endpoints for managing alerts

### 9. No Health Check Endpoint
- **Fix:** Add a `/health` endpoint returning `200 OK` with optional status details. Essential for Docker health checks, load balancers, and monitoring.

### 10. No CORS Configuration
- If the frontend is ever served from a different origin (or during development), requests will be blocked.
- **Fix:** Add `tower-http::cors::CorsLayer` with configurable allowed origins.

---

## Medium Priority (Should Improve)

### 11. No API Versioning
- WebSocket protocols are fragile to change. Without versioning, breaking changes will break all clients.
- **Fix:** Prefix routes with a version (`/api/v1/ws/sentinel`) or include a protocol version in the handshake.

### 12. Weak Data Model Validation
- **File:** `src/schema.rs` — `SentinelAlert` fields have no constraints (e.g., `performance` and `expected` are raw `i32`, `id` is an unvalidated `String`).
- **Fix:** Add validation using `validator` crate or custom `TryFrom` implementations. Consider using non-negative types for performance/expected values.

### 13. No Rate Limiting
- WebSocket endpoints have no rate limiting — a single client could flood the broadcast channel.
- **Fix:** Add a rate-limiting middleware (e.g., `tower-rate-limit` or a custom layer).

### 14. Inconsistent Error Handling Patterns
- Some errors are logged and broken from (`sentinel_ws.rs`), others are silently ignored (`websockets.rs`). No unified error response type.
- **Fix:** Define an `AppError` enum with variants for different failure modes, and use a custom `ErrorHandler` layer in Axum.

### 15. Missing Unit / Integration Tests
- Zero tests exist. The broadcast logic, alert management, and WebSocket handling are all untested.
- **Fix:** Add:
  - Unit tests for schema validation
  - Integration tests using `axum::Router::new().into_service()` to test endpoints
  - Mock-based tests for the alert service layer

---

## Low Priority (Nice to Have)

### 16. No Metrics / Observability
- Basic tracing is set up, but there are no application metrics.
- **Fix:** Add `axum-prometheus` or OpenTelemetry for:
  - Active WebSocket connections count
  - Messages sent/received rate
  - Alert creation/removal counters
  - Response time histograms

### 17. No Dockerfile / Containerization
- **Fix:** Create a multi-stage Dockerfile to minimize the final image size (compile in stage 1, run with musl in stage 2).

### 18. No CI/CD Pipeline
- **Fix:** Add GitHub Actions for:
  - `cargo check`, `cargo clippy`, `cargo test` on every PR
  - Build and deploy on main branch merge

### 19. Improve AlertType Semantics
- **File:** `src/schema.rs:L4` — The `AlertType` enum has `Update`, `New`, `Remove`. Consider renaming to more descriptive variants or adding documentation explaining when each is emitted.

### 20. Separate Frontend Embedding from Backend Logic
- The hardcoded relative path and tight coupling between frontend build output and backend make the build process fragile.
- **Fix:** Use a build script (`build.rs`) to verify the frontend exists at build time, or support an external static directory as a fallback when `rust-embed` assets aren't found.

---

## Suggested Architecture for v2

```
backend/
├── src/
│   ├── main.rs            # Entry point, config loading, server setup
│   ├── config.rs          # Configuration struct + loader
│   ├── error.rs           # Unified AppError type + handler
│   ├── state.rs           # AppState with typed service accessors
│   ├── schema/
│   │   ├── mod.rs
│   │   └── alert.rs       # SentinelAlert, AlertType, validation
│   ├── handlers/
│   │   ├── mod.rs
│   │   ├── sentinel_ws.rs  # Sentinel WebSocket handler only
│   │   └── health.rs       # Health check endpoint
│   ├── services/
│   │   ├── mod.rs
│   │   ├── alert_service.rs  # Trait + in-memory impl (later DB impl)
│   │   └── broadcast.rs      # Connection manager for WebSocket clients
│   └── middleware/
│       ├── mod.rs
│       └── auth.rs           # Auth middleware
├── tests/
│   └── integration_tests.rs
├── build.rs                # Verify frontend output exists
└── Cargo.toml
```

---

## Recommended Priority Order for Execution

| Phase | Tasks | Effort |
|-------|-------|--------|
| **Phase 1 — Stabilize** | #1, #2, #3, #4, #5 | Medium |
| **Phase 2 — Secure** | #6, #9, #10, #13 | Medium |
| **Phase 3 — Persist** | #8, #7, #11 | High |
| **Phase 4 — Harden** | #12, #14, #15 | Medium |
| **Phase 5 — Operate** | #16, #17, #18 | Low-Medium
