// Wire contracts matched to the Phase 2 backend (see docs/phase2.md §0.2).

/** Alert lifecycle event type. */
export type AlertType = 'New' | 'Update' | 'Remove';

/** Evaluated health status of a monitoring target. */
export type ServiceStatus = 'healthy' | 'degraded' | 'down';

/** GIS service type (backend stores/serializes these exact strings). */
export type ServiceType = 'WMS' | 'WFS' | 'WMTS' | 'OAF' | 'ArcGIS_REST' | 'HTTP';

/** HTTP method used when probing a target. */
export type HttpMethod = 'GET' | 'POST';

/** A single alert lifecycle event pushed over `/ws/sentinel`. */
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
    /** ISO 8601 timestamp. */
    triggered_at: string;
}

/** First message on connect: full list of open alerts (replaces local state). */
export interface WsSnapshot {
    type: 'snapshot';
    alerts: AlertEvent[];
}

/**
 * Live lifecycle event. The backend uses a serde internally-tagged enum, so
 * the `AlertEvent` fields are FLATTENED into the message object:
 * `{"type":"alert","alert_id":12,...}` — not nested under a key.
 */
export type WsAlert = { type: 'alert' } & AlertEvent;

export type WsMessage = WsSnapshot | WsAlert;

/** Transport/connection state surfaced to the UI. */
export type ConnectionState = 'disconnected' | 'connecting' | 'connected';

// ---------------------------------------------------------------------------
// REST contracts (`/api/v1/alert-points`)
// ---------------------------------------------------------------------------

export type AuthConfig = BasicAuth | BearerAuth | HeaderAuth;

export interface BasicAuth {
    type: 'basic';
    username: string;
    password?: string;
}

export interface BearerAuth {
    type: 'bearer';
    token: string;
}

export interface HeaderAuth {
    type: 'header';
    name: string;
    value: string;
}

/** A configured monitoring target. */
export interface AlertPoint {
    id: string;
    name: string;
    url: string;
    service_type: ServiceType;
    check_interval_seconds: number;
    expected_response_time_ms: number;
    http_method: HttpMethod;
    /** Object of `{ "Header-Name": "value" }`. */
    custom_headers: Record<string, string>;
    auth_config: AuthConfig | null;
    enabled: boolean;
    /** ISO 8601. */
    created_at: string;
    /** ISO 8601. */
    updated_at: string;
    /** ISO 8601 or null when never probed. */
    last_checked_at: string | null;
}

/** Payload for creating an alert point (all server-side defaults applied). */
export interface NewAlertPoint {
    name: string;
    url: string;
    service_type: ServiceType;
    check_interval_seconds: number;
    expected_response_time_ms: number;
    http_method?: HttpMethod;
    custom_headers?: Record<string, string>;
    auth_config?: AuthConfig | null;
    enabled?: boolean;
}

/** Partial update payload — only set fields change. */
export type AlertPointPatch = Partial<Omit<NewAlertPoint, 'auth_config'>> & {
    auth_config?: AuthConfig | null;
};

/** Pagination envelope returned by the list endpoint. */
export interface Pagination {
    page: number;
    per_page: number;
    total: number;
}

export interface AlertPointList {
    data: AlertPoint[];
    pagination: Pagination;
}

/** Detail response: the point plus its recent probe history. */
export interface AlertPointDetail extends AlertPoint {
    recent_probes: ProbeResult[];
}

/** Persisted probe execution result. */
export interface ProbeResult {
    id: number;
    alert_point_id: string;
    /** ISO 8601. */
    timestamp: string;
    response_time_ms: number | null;
    status_code: number | null;
    is_up: boolean;
    error_message: string | null;
    raw_response_snippet: string | null;
}

/** One point of the graph series in `GET /{id}/history` (minimal payload). */
export interface ProbeHistoryPoint {
    /** ISO 8601. */
    timestamp: string;
    response_time_ms: number | null;
    is_up: boolean;
}

/** A failed probe with error detail (`GET /{id}/incidents`, CSV/JSON export). */
export interface ProbeIncident {
    /** ISO 8601. */
    timestamp: string;
    response_time_ms: number | null;
    status_code: number | null;
    is_up: boolean;
    error_message: string | null;
}

/** Result of an on-demand `/test` probe (not persisted). */
export interface ProbeObservation {
    status_code: number | null;
    response_time_ms: number | null;
    is_up: boolean;
    error_message: string | null;
    raw_response_snippet: string | null;
}

/** Response of `GET /api/v1/alert-points/{id}/history` (task 3.5). */
export interface ProbeHistory {
    hours: number;
    uptime_pct: number | null;
    probes: ProbeHistoryPoint[];
}

/** Uniform backend error body: `{ error: { code, message, details } }`. */
export interface ApiErrorBody {
    error: {
        code: string;
        message: string;
        details: Record<string, string> | null;
    };
}
