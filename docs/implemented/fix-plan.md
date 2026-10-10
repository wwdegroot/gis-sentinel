# Fix Plan — Findings from `docs/findings.md`

Decisions (confirmed with user):
- History response: graph fields only (`timestamp`, `response_time_ms`, `is_up`).
- Export: backend endpoints (`GET /{id}/history/export?format=csv|json`).
- Delete/disable: resolve alerts + broadcast `Remove`; no Redis queue purge.

## Finding 1: Disabled/deleted service points stay in active incidents

Root causes:
- `POST /{id}/update {enabled:false}` never resolves the point's open alert → stuck incident.
- `POST /{id}/delete` cascades `active_alerts` rows in the DB but broadcasts nothing, so
  connected WebSocket clients keep the incident card until reconnect.

Changes (backend):
1. `handlers/alert_points_api.rs` — `update_alert_point`: on `enabled` → `false`, fetch the
   open alert, `repo::active_alerts::resolve()`, broadcast a `Remove` `AlertEvent`.
2. `handlers/alert_points_api.rs` — `delete_alert_point`: fetch point + open alert before
   delete, delete, broadcast `Remove` with the point metadata.
3. Add a broadcast helper in the handler layer (factor out of `probe_worker::broadcast_event`).
4. `workers/probe_worker.rs` — hardening: skip jobs for missing/disabled points gracefully
   (no error spam, no alert rows); reset evaluator streak for that target.
5. Frontend: verify `sentinelSocket` / dashboard drop cards on `alert_type: 'Remove'`.
6. Tests: integration tests — disable/delete with an open alert resolves it and emits `Remove`.

## Finding 2: Slim history response (graph fields only)

1. `db/repo/probe_results.rs` — new `ProbeHistoryPoint { timestamp, response_time_ms, is_up }`;
   `list_since` selects only those columns; `ProbeHistory` becomes
   `{ hours, uptime_pct, probes: Vec<ProbeHistoryPoint> }`. Keep `HISTORY_ROW_CAP`.
2. `handlers/alert_points_api.rs` — `get_history` returns the new shape.
   `GET /{id}` detail keeps full `ProbeResult` in `recent_probes` (detail view, not the graph).
3. Frontend:
   - `types.ts`: add `ProbeHistoryPoint`; `ProbeHistory.probes` uses it.
   - `LatencyChart.svelte`: verify (already only uses timestamp/latency/is_up).
   - `IncidentTable.svelte`: stop deriving incidents from history probes; fetch from the
     dedicated incidents endpoint (Finding 4) so `error_message`/`status_code` are preserved.

> Note: this does NOT affect live dashboard incidents. Those flow exclusively through the
> WebSocket (DB snapshot on connect + live `Alert` events) and are untouched by this plan.
> The history-page `IncidentTable` shows *historical failed probes*, not active incidents.

## Finding 3a: Dedicated incidents endpoint (history page table)

1. New route `GET /api/v1/alert-points/{id}/incidents?hours=` — failed probes only
   (`is_up = false`) with full detail: `{timestamp, response_time_ms, status_code,
   error_message}`. Purpose-built data source for the history-page `IncidentTable`.
2. Repo: new query in `db/repo/probe_results.rs` (`list_failed_since`), bounded by row cap,
   `hours` clamped to [1, 720]; 404 for unknown point.
3. Frontend: `IncidentTable` (or `history/+page.svelte`) fetches this endpoint per selected
   point/window instead of deriving incidents from the slim history response.

## Finding 3b: CSV/JSON export via backend endpoints (downloads only)

1. New route `GET /api/v1/alert-points/{id}/history/export?format=csv|json&hours=`
   - `csv` → `200 text/csv`, header `timestamp,response_time_ms,status_code,is_up,error_message`,
     rows quoted/escaped.
   - `json` → `200 application/json`, array of `{timestamp, response_time_ms, status_code,
     is_up, error_message}` (no `raw_response_snippet`).
   - `hours` clamped to [1, 720]; 404 for unknown point; bounded by row cap.
2. Sole purpose: file downloads via the export buttons in `IncidentTable`.
   It is NOT a data source for any view — the table uses Finding 3a's endpoint.
3. Frontend: `api.ts`: add `exportHistoryUrl(id, format, hours)` download helper.

## Order of work & verification

| Step | Scope | Verify with |
|---|---|---|
| 1 | Finding 1 | `cargo test` integration tests; manual WS check |
| 2 | Finding 2 | `SQLX_OFFLINE=true cargo build`; frontend type-check |
| 3 | Findings 3a + 3b | `curl` incidents + both export formats; UI click-through |
| 4 | Full pass | `cargo test`, `pnpm check && pnpm build`, smoke test against `postgres-gs` |
