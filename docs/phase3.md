# Phase 3 — Frontend Development & UI/UX (Change Plan)

Covers `tasks.md` → Phase 3: **3.1 WebSocket Client** · **3.2 Layout & Navigation** · **3.3 Live Dashboard** · **3.4 Alert Points Config UI** · **3.5 History & Service Detail**

Status: ✅ **PHASE 3 COMPLETE** — 3.1–3.5 all implemented & verified 2026-09-18. Remaining: Playwright E2E (task 5.2), optional follow-ups (per-target probe timeout column, URL-synced history selection, Redis Pub/Sub fan-out).

---

## 0. Current State (what Phase 3 builds on)

### 0.1 Frontend inventory

| File | State |
|---|---|
| `src/lib/sentinelSocket.svelte.ts` | Demo-era class: `adress` typo, no reconnect, `connection` boolean only, parses old `SentinelAlert` shape (`atype`, `performance`, `expected`, `up`), contains a `test()` mock-data generator |
| `src/lib/types.ts` | Old `AlertType`/`SentinelAlert` enums — **do not match backend** |
| `src/routes/+page.svelte` | Raw WebSocket echo playground (`/ws` route — deleted from backend in 2.4!) — must be replaced |
| `src/routes/sentinel/+page.svelte` | Prototype dashboard: dumps every alert as raw key-value rows, color by `performance > expected` |
| `src/routes/+layout.svelte` | Bare `{@render children()}` — no nav |
| `src/demo.spec.ts` | Placeholder vitest sum test |
| `vite.config.ts` | Dev proxy already forwards `/ws` (ws: true) and `/api` to `BACKEND_PORT` (default 3000) |
| `svelte.config.js` | `adapter-static` with `fallback: '200.html'`, `ssr = false` in root `+layout.ts` — backend serves this build via rust-embed |

### 0.2 Backend contracts to code against (verified in Phase 2)

**WebSocket** (`/ws/sentinel`, serde internally-tagged enum — event fields are *flattened* into the message object):

```jsonc
// first message on connect — replaces the whole list
{"type": "snapshot", "alerts": [ { ...AlertEvent } ]}

// live lifecycle event — fields at the TOP level (not nested under "alert")
{"type": "alert", "alert_id": 12, "alert_point_id": "01a0…", "alert_type": "New",
 "name": "GeoServer Demo", "url": "https://…", "service_type": "WMS",
 "status": "down", "reason": "connection failed: …",
 "response_time_ms": null, "expected_response_time_ms": 500,
 "triggered_at": "2026-09-18T15:43:39.773Z"}
```

`alert_type`: `New` | `Update` | `Remove` · `status`: `healthy` | `degraded` | `down` · `service_type`: `WMS` | `WFS` | `WMTS` | `OAF` | `ArcGIS_REST` | `HTTP`

Semantics: `New` → add card; `Update` → replace card with matching `alert_id`; `Remove` → drop card with matching `alert_id`. A reconnect always starts with a fresh `Snapshot`, which is also the re-sync mechanism after a dropped connection.

**REST** (GET/POST only, mounted under `/api/v1/alert-points`):

| Method & path | Purpose | Success | Errors |
|---|---|---|---|
| `GET /api/v1/alert-points` | list; `?enabled=&service_type=&page=&per_page=` | `{ data: AlertPoint[], pagination: { page, per_page, total } }` | — |
| `POST /api/v1/alert-points` | create | 201 + `AlertPoint` | 422 `{error:{code,message,details:{field:msg}}}` |
| `GET /api/v1/alert-points/{id}` | detail + history | `{ ...AlertPoint, recent_probes: ProbeResult[] }` | 404 |
| `POST /api/v1/alert-points/{id}/update` | partial update (only sent fields change) | `AlertPoint` | 400/404/422 |
| `POST /api/v1/alert-points/{id}/delete` | delete (cascades) | 204 | 404 |
| `POST /api/v1/alert-points/{id}/test` | on-demand probe | `ProbeObservation` | 400/404 |

`AlertPoint`: `{ id, name, url, service_type, check_interval_seconds, expected_response_time_ms, http_method, custom_headers (object), auth_config (object|null), enabled, created_at, updated_at, last_checked_at }`.
`ProbeResult`/`ProbeObservation`: `{ status_code|null, response_time_ms|null, is_up, error_message|null, raw_response_snippet|null }` (persisted rows add `id`, `alert_point_id`, `timestamp`).

**Validation rules mirrored from backend** (client-side pre-validation for fast feedback): absolute `http(s)` URL without embedded credentials; `check_interval_seconds` 10–86 400; `expected_response_time_ms` 1–600 000; `custom_headers` = object of string values without hop-by-hop headers; `auth_config` = `{type:"basic",username,password?}` | `{type:"bearer",token}` | `{type:"header",name,value}`.

---

## 1. Task 3.1 — WebSocket Client Improvements (`sentinelSocket.svelte.ts`) ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):** built as planned.
> - `types.ts` fully rewritten to the Phase 2 contracts (`AlertEvent`, `WsMessage`,
>   `AlertPoint`, `ProbeResult`, `AuthConfig`, `ApiErrorBody`, …).
> - `sentinelSocket.svelte.ts` rewritten: `resolveWebSocketUrl()` (wss on https),
>   snapshot-replace + New/Update/Remove keyed by `alert_id` (New idempotent),
>   malformed messages swallowed, backoff 500 ms→30 s + jitter via
>   `computeBackoffDelay()` (pure, unit-tested), stale-socket guards, `MAX_ALERTS`
>   cap, singleton export `sentinel`; `test()` mock deleted.
> - 12 vitest cases: message state machine, backoff math, reconnect scheduling
>   with fake timers + stubbed WebSocket (incl. user-close and stale-socket cases),
>   URL resolution.
> - Env fixes: `npm ci` failed on peer conflicts → `npm install --legacy-peer-deps`;
>   added `@types/node` (vite.config.ts `process` typing).
> - `/sentinel` page minimally adapted to the new store (keyed `{#each}`, status-
>   based colors, reconnect badge) — full redesign still lands in 3.3.
> - **vite 8 dev-proxy fix:** `/ws` upgrade failed with close code 1006 until
>   `rewriteWsOrigin: true` was added to the proxy config — verified live
>   (`ws://localhost:5173/ws/sentinel` → snapshot through the proxy).
> - Checks: svelte-check 0 errors · prettier clean · vitest 12/12 · `vite build` OK.

### 1.1 New types (`src/lib/types.ts` — full rewrite)

```ts
export type AlertType = 'New' | 'Update' | 'Remove';
export type ServiceStatus = 'healthy' | 'degraded' | 'down';
export type ServiceType = 'WMS' | 'WFS' | 'WMTS' | 'OAF' | 'ArcGIS_REST' | 'HTTP';

export interface AlertEvent {
    alert_id: number;
    alert_point_id: string;
    alert_type: AlertType;
    name: string;
    url: string;
    service_type: ServiceType;
    status: ServiceStatus;
    reason: string;
    response_time_ms: number | null;
    expected_response_time_ms: number;
    triggered_at: string; // ISO 8601
}

export interface WsSnapshot { type: 'snapshot'; alerts: AlertEvent[] }
export interface WsAlert { type: 'alert' } & AlertEvent;
export type WsMessage = WsSnapshot | WsAlert;

export type ConnectionState = 'disconnected' | 'connecting' | 'connected';
```

*(Keep `SentinelAlert` removed; the old `performance`/`expected`/`up` fields no longer exist — the card derives up/down from `status` and latency from `response_time_ms` vs `expected_response_time_ms`.)*

### 1.2 Rewritten store (same file name, new class)

- **Fix the typo**: `adress` → `address`.
- **URL resolution**: constructor takes no URL; compute `(location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + location.host + '/ws/sentinel'` at connect time (works same-origin in prod; vite proxy in dev).
- **Message handling** (`processMessage`):
  - parse once as `WsMessage`; `type === 'snapshot'` → `alerts = snapshot.alerts` (full replace); `type === 'alert'` → switch on `alert_type`:
    - `New` → replace-by-`alert_id` if present (defensive vs duplicate), else push;
    - `Update` → replace matching `alert_id`, else ignore (log);
    - `Remove` → filter out matching `alert_id`.
  - invalid JSON → `console.error` and continue (never throw in the listener).
- **Reconnect with exponential backoff**: on `close`/`error`, schedule reconnect after `min(base * 2^n + jitter, max)` (base 500 ms, max 30 s), reset `n` on successful open; expose `retryCount`/`nextRetryIn` for UI; `close()` (user-initiated) stops the loop permanently.
- **Connection state**: `connection: ConnectionState = $state(...)` — `connecting` from `connect()` until `open`, `connected` on `open`, `reconnecting` when a reconnect is scheduled (surface via a getter so the nav badge can show 🟡/🟢/🔴).
- **Derived stats** for the dashboard header: `totalAlerts`, `countsByStatus` (`{ down, degraded }`), computed from `alerts`.
- **Delete** `test()` and the mock generator.
- **Singleton export**: `export const sentinel = new SentinelSocket();` — shared across routes so the nav connection badge and the dashboard see the same socket; `connect()` is idempotent (no-op if already open/connecting).

### 1.3 Tests (`src/lib/sentinelSocket.test.ts`, vitest)

- Replace `demo.spec.ts`.
- Unit-test `processMessage` in isolation (feed synthetic `MessageEvent`s): snapshot replace, New add, Update-by-id, Remove-by-id, unknown type ignored, malformed JSON swallowed.
- Unit-test backoff scheduler with fake timers (vitest `vi.useFakeTimers`): 500 ms → 1 s → 2 s … capped at 30 s, reset on open.

---

## 2. Task 3.2 — Application Layout & Navigation ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):**
> - `app.css`: `@custom-variant dark` (class strategy) + `@theme inline` mapping
>   `--color-surface/-surface-2/-line/-text/-muted` to runtime CSS vars switched
>   by `.dark` on `<html>` — utilities like `bg-surface` re-theme instantly.
> - `app.html`: pre-paint script defaults to dark, honors stored `sentinel-theme`.
> - `theme.svelte.ts` store (toggle + persist + apply); toggle in nav and on `/settings`.
> - `+layout.svelte`: nav (Dashboard/Services/History/Settings, active highlight via
>   `$app/state` page), `ConnectionBadge`, theme toggle; socket connected in layout
>   (Q4-b) so the badge is live app-wide. Mobile second nav row.
> - Routes: `/` = dashboard (3.3); `/sentinel/+page.ts` redirect (prerendered as
>   client-side redirect since `ssr = false`); `/services`, `/history`, `/settings`
>   stubs created.

## 3. Task 3.3 — Live Sentinel Monitoring Dashboard (`/`) ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):**
> - Components: `StatusBadge`, `ServiceBadge` (underscores→spaces), `LatencyMeter`
>   (0–2× SLA track, emerald/amber/rose by ratio), `AlertCard` (severity accent
>   border, ticking outage duration, reason, SLA meter), `AlertsEmpty`
>   (connecting skeleton vs all-clear), `SummaryStats` (total via
>   `GET /alert-points?per_page=1` → `pagination.total`, healthy = total −
>   incidents, avg latency over reported alerts, 30 s refresh).
> - `routes/+page.svelte`: status/service-type/search filters (client-side
>   `$derived`), severity-then-recency sort, keyed `{#each}` by `alert_id`,
>   "no matches" vs empty-state distinction, clear-filters button.
> - New `api.ts` request wrapper with `ApiError` parsing the backend error body
>   (extended for CRUD in 3.4); `format.ts` helpers.
> - Verified live: backend serves the prerendered shell + chunks containing the
>   new dashboard (all 4 markers found), REST 200, theme pre-paint script
>   present, `/sentinel` + `/services` reachable; svelte-check 0/0, prettier
>   clean, vitest 12/12, `vite build` OK.

### 3.1 Components (`src/lib/components/`)

| Component | Responsibility |
|---|---|
| `SummaryStats.svelte` | Header tiles: **Total monitored** (needs `GET /alert-points?per_page=1` → `pagination.total`), **Healthy** (total − alerts), **Active incidents** (`sentinel.totalAlerts`), **Avg latency** (mean of `response_time_ms` across open alerts; "—" when none) |
| `AlertCard.svelte` | One open alert: name + service-type badge (`WMS`/`WFS`/…/`ArcGIS REST`), URL (truncated, `title` attr full), status badge, latency meter (`response_time_ms` vs `expected_response_time_ms` → ratio, red at >1×), reason/error text, incident duration (`triggered_at` → now, ticking via `$interval`) |
| `StatusBadge.svelte` | Colored pill for `healthy`/`degraded`/`down` (green/amber/rose) — reused in cards, tables, history |
| `ServiceBadge.svelte` | Pill for service type (normalizes `ArcGIS_REST` → `ArcGIS REST`) |
| `LatencyMeter.svelte` | Horizontal bar: actual vs expected SLA; scales at 2× expected max |
| `AlertsEmpty.svelte` | Empty state ("No active incidents — all services healthy") vs "connecting" skeleton |

### 3.2 Page composition (`routes/+page.svelte`)

- Mounts the singleton socket (`onMount` → `sentinel.connect()`; `onDestroy` does **not** close — the socket is app-global, only navigations away from the whole app end it).
- Renders `SummaryStats` + responsive grid of `AlertCard`s filtered/sorted client-side.
- **Filters row**: by status (`all | down | degraded`), by service type (dropdown of `ServiceType`), free-text search on `name`/`url` — all client-side over `sentinel.alerts` (Svelte 5 `$derived`).

---

## 4. Task 3.4 — Alert Points Configuration Interface (`/services`) ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):**
> - `api.ts` extended with `getAlertPoint`, `createAlertPoint`, `updateAlertPoint`,
>   `deleteAlertPoint`, `testAlertPoint` (GET/POST scheme; 204 handled).
> - New `validation.ts`: client-side rules mirroring the backend (URL, interval
>   10–86400, SLA 1–600 000 ms, header-name token check + hop-by-hop denylist,
>   auth shapes) + `AlertPointForm` ⇄ payload mapping (`formToPayload`/
>   `pointToForm`/`emptyForm`); 12 vitest cases.
> - Components: `AlertPointModal` (create/edit, key-value header rows, auth kind
>   switcher, server 422 details merged into field errors), `ServiceTable`
>   (optimistic enable toggle with rollback, relative last-check),
>   `TestResultPanel`, `DeleteConfirm`.
> - Page: loading skeleton, load-error retry, empty state, action error banner;
>   create/edit/delete refetch, toggle is optimistic.
> - Note: the form intentionally has no per-target timeout field — the backend
>   `alert_points` table has no timeout column (worker uses
>   `PROBE_TIMEOUT_MS_DEFAULT`); tracked as a small backend follow-up.
> - Verified: 24 vitest tests, svelte-check 0/0, prettier, build; live REST
>   round-trip with form-shaped payloads (create → toggle-update → detail →
>   on-demand test → delete) and 422 details passthrough; `/services` route chunk
>   served by the backend (nodes/5.*.js → 200).

### 4.1 API client (`src/lib/api.ts` — new)

- Thin typed wrappers: `listAlertPoints(params)`, `createAlertPoint(payload)`, `getAlertPoint(id)`, `updateAlertPoint(id, patch)`, `deleteAlertPoint(id)`, `testAlertPoint(id)` — `fetch` + JSON, throwing `ApiError` (parsed from the backend `{error:{code,message,details}}` shape).
- Shared `AlertPoint`, `ProbeResult`, `Pagination` interfaces live in `types.ts`.

### 4.2 Page & components

| Component | Responsibility |
|---|---|
| `routes/services/+page.svelte` | Loads list on mount (`per_page=200`); renders `ServiceTable`; holds modal state; refresh button |
| `ServiceTable.svelte` | Columns: Name (+url), Type (`ServiceBadge`), Interval, SLA, Enabled toggle (optimistic `POST /{id}/update {enabled}`, revert on error), last check (`last_checked_at` relative), row actions: **Test now** / **Edit** / **Delete** |
| `AlertPointModal.svelte` | Create + edit form (same fields): name, url, service_type select, check interval, expected SLA ms, HTTP method, custom headers (key/value rows → object), auth (none/basic/bearer/header, fields per type), enabled checkbox. Client-side validation mirroring backend rules (see §0.2); server `422` details merged into field errors |
| `TestResultPanel.svelte` | "Test now" → `POST /{id}/test`; shows `is_up`, status code, latency, snippet (collapsible) inline; disabled while running |
| `DeleteConfirm.svelte` | Confirm dialog: "Delete `name`? Probe history & alerts will be removed." → `POST /{id}/delete` |

- After create/update/delete: refetch list; the dashboard reflects alerts automatically via the socket (no coupling needed).

---

## 5. Task 3.5 — Historical Analytics & Service Detail View ✅ IMPLEMENTED

> **Implementation notes (2026-09-18):**
> - **Backend prerequisite delivered with this task** (plan §5.1):
>   `GET /api/v1/alert-points/{id}/history?hours=` →
>   `{ hours, uptime_pct, probes }` (`ProbeHistory`). New repo fns
>   `probe_results::list_since` (capped at `HISTORY_ROW_CAP = 5000`, oldest-first)
>   and the existing `uptime_percentage` combined in `get_history`; `hours`
>   clamped to [1, 720]. `.sqlx` cache regenerated (18 queries).
> - `LatencyChart.svelte`: hand-rolled SVG (Q2-a) — x spans the whole window
>   (gaps visible), y spans 0..max(observed, 1.25×SLA), dashed SLA line,
>   red baseline marks for failed probes, `<title>` tooltips, grid + ms labels.
> - `UptimeStats.svelte`: 24 h / 7 d / 30 d tiles via `Promise.all` history
>   fetches; `$effect` refetches on point change; `n/a` for empty windows.
> - `IncidentTable.svelte`: failed probes with sticky header, CSV/JSON export
>   via blob download.
> - `/history` page: point selector + window selector (24 h/7 d/30 d), per-point
>   detail (SLA line from the point's `expected_response_time_ms`), row-cap note,
>   empty/loading/error states. URL-sync of `?point=` deferred (Q in plan) —
>   selection is client-side state.
> - Verified live: history endpoint (uptime 100.0, 4 probes after ~45 s of
>   10 s-interval probing, `hours` clamp, unknown-id 404); `/history` chunk
>   served by the backend (nodes/3.*.js → 200); 24 vitest tests, svelte-check
>   0/0, prettier, `vite build` OK.

### 5.1 ⚠️ Backend prerequisite (small Phase-2.5 addendum, must land with 3.5)

The repo layer already has everything; only an HTTP route is missing:

- `GET /api/v1/alert-points/{id}/history?hours=24` → `Vec<ProbeResult>` (new repo fn `probe_results::list_since(pool, id, hours)` — `timestamp > now() - make_interval(hours => $2)`, ascending; reuse `.sqlx` prepare flow).
- Optionally `GET /api/v1/alert-points/history/summary?hours=24` for per-point uptime% + avg latency in one call (mirrors `uptime_percentage`; else compute client-side from the detail endpoint's `recent_probes` — acceptable for 24 h, **not** for 7 d/30 d windows at 10 s intervals → implement `list_since` with a row cap, e.g. 5 000).

### 5.2 Frontend

| Component | Responsibility |
|---|---|
| `routes/history/+page.svelte` | List of alert points (reuse API client); select one → detail |
| `LatencyChart.svelte` | **Hand-rolled SVG line chart** (no chart library — keep deps at zero; x = time, y = `response_time_ms`, SLA line at `expected_response_time_ms`, gaps where `is_up = false` colored red) — see Open Question Q2 |
| `UptimeStats.svelte` | Uptime % over 24 h / 7 d / 30 d (from `history` + summary endpoint) |
| `IncidentTable.svelte` | Failed probes + open-alert history rows (timestamp, status, error snippet), export button → client-side CSV/JSON blob download |

- Service detail view: reuse on `/history?point={id}` — chart + stats + incident table for one target.

---

## 6. File-by-file change summary

| File | Action |
|---|---|
| `src/lib/types.ts` | **REWRITE** — backend-matched `AlertEvent`, `AlertPoint`, `ProbeResult`, enums, `WsMessage`, `ConnectionState` |
| `src/lib/sentinelSocket.svelte.ts` | **REWRITE** — fixed `address`, dynamic `ws/wss` URL, snapshot + flattened-alert handling, exponential backoff, `ConnectionState`, singleton export; `test()` deleted |
| `src/lib/api.ts` | **NEW** — typed REST client + `ApiError` |
| `src/lib/index.ts` | re-export store/api for `$lib` ergonomics |
| `src/lib/components/{SummaryStats,AlertCard,StatusBadge,ServiceBadge,LatencyMeter,AlertsEmpty,ServiceTable,AlertPointModal,TestResultPanel,DeleteConfirm,LatencyChart,UptimeStats,IncidentTable}.svelte` | **NEW** |
| `src/routes/+layout.svelte` | **REWRITE** — nav bar, connection badge, theme toggle |
| `src/app.css` | dark-mode `@custom-variant`, `@theme` color tokens, form-control base styles |
| `src/app.html` | pre-paint theme script |
| `src/routes/+page.svelte` | **REWRITE** — dashboard (3.3); echo demo deleted |
| `src/routes/sentinel/+page.ts` | **REWRITE** → `redirect(307, '/')` |
| `src/routes/services/{+page.svelte}` | **NEW** (3.4) |
| `src/routes/history/{+page.svelte}` | **NEW** (3.5) |
| `src/routes/settings/+page.svelte` | **NEW** stub |
| `src/demo.spec.ts` | **DELETE** → `src/lib/sentinelSocket.test.ts` (+ component tests where cheap) |
| backend: `handlers/alert_points_api.rs` + `db/repo/probe_results.rs` + `.sqlx` cache | **EXTEND** — `GET /{id}/history` (prerequisite for 3.5, ~30 lines) |

Dependencies: **none added** (hand-rolled SVG chart; icons via inline SVG snippets).

---

## 7. Open Questions

| # | Question | Options | Recommendation |
|---|---|---|---|
| Q1 | `/sentinel` route | a) redirect to `/` (recommended), b) keep separate full dashboard | **a** — one dashboard, `/` is what users expect |
| Q2 | Charting for 3.5 | a) hand-rolled SVG (zero deps, fits scale of data, full theming control), b) Chart.js/Flayer, c) ECharts | **a** for Phase 3; revisit if 30-day zoom/pan is demanded |
| Q3 | Dark mode default | a) dark (monitoring-tool convention), b) light, c) `prefers-color-scheme` | **a**, toggle persisted |
| Q4 | Where the WS singleton connects | a) connect lazily on dashboard mount (nav badge "—" elsewhere), b) connect in root layout (badge live everywhere) | **b** — badge is a feature; reconnect loop makes it safe |
| Q5 | Optimistic UI depth in `/services` | a) optimistic toggle/delete with rollback on error (recommended), b) await-then-refresh only | **a** for the toggle, **b** for delete (cascades — cheap to just refetch) |

---

## 8. Verification plan (mirrors phase2.md style)

1. `pnpm check` (svelte-check) + `pnpm lint` (prettier) clean; `pnpm test` green (socket state machine + backoff under fake timers; form validation unit tests).
2. Manual E2E against the live stack (backend `cargo run`, frontend `pnpm dev` with proxy — plus one pass against the rust-embed'd static build on `:3000`):
   - `/` renders snapshot from DB; kill a target → `New` card appears live; recover → card disappears; restart backend mid-session → reconnect badge cycles 🟡→🟢 and list re-syncs;
   - `/services`: create (validation errors shown), edit, toggle enable, Test-now panel, delete with cascade;
   - `/history`: chart + uptime + CSV export for a target with probe data;
   - theme toggle persists across reload.
3. Playwright E2E (task 5.2) explicitly deferred — not in Phase 3 scope.
