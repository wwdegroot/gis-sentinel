/**
 * Typed REST client for `/api/v1/alert-points` (task 3.3/3.4).
 *
 * Errors: non-2xx responses throw `ApiError` parsed from the backend's
 * uniform `{ error: { code, message, details } }` body.
 */

import type {
    AlertPoint,
    AlertPointDetail,
    AlertPointList,
    AlertPointPatch,
    ProbeHistory,
    NewAlertPoint,
    ProbeIncident,
    ProbeObservation,
    ServiceType
} from './types';

export class ApiError extends Error {
    constructor(
        public status: number,
        public code: string,
        message: string,
        public details: Record<string, string> | null
    ) {
        super(message);
        this.name = 'ApiError';
    }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
    const res = await fetch(path, {
        ...init,
        headers: { 'content-type': 'application/json', ...init?.headers }
    });

    if (!res.ok) {
        let code = 'http_error';
        let message = res.statusText || `HTTP ${res.status}`;
        let details: Record<string, string> | null = null;
        try {
            const body = await res.json();
            if (body?.error) {
                code = body.error.code ?? code;
                message = body.error.message ?? message;
                details = body.error.details ?? null;
            }
        } catch {
            // non-JSON error body — keep the HTTP fallbacks
        }
        throw new ApiError(res.status, code, message, details);
    }

    if (res.status === 204) return undefined as T;
    return res.json() as Promise<T>;
}

export interface ListAlertPointsParams {
    enabled?: boolean;
    service_type?: ServiceType;
    page?: number;
    per_page?: number;
}

/** GET /api/v1/alert-points — list with optional filtering/pagination. */
export function listAlertPoints(params: ListAlertPointsParams = {}): Promise<AlertPointList> {
    const query = new URLSearchParams();
    if (params.enabled !== undefined) query.set('enabled', String(params.enabled));
    if (params.service_type) query.set('service_type', params.service_type);
    if (params.page !== undefined) query.set('page', String(params.page));
    if (params.per_page !== undefined) query.set('per_page', String(params.per_page));
    const qs = query.toString();
    return request(`/api/v1/alert-points${qs ? `?${qs}` : ''}`);
}

/** GET /api/v1/alert-points/{id} — detail + recent probe history. */
export function getAlertPoint(id: string): Promise<AlertPointDetail> {
    return request(`/api/v1/alert-points/${id}`);
}

/** POST /api/v1/alert-points — create. Returns the created point (201). */
export function createAlertPoint(payload: NewAlertPoint): Promise<AlertPoint> {
    return request('/api/v1/alert-points', { method: 'POST', body: JSON.stringify(payload) });
}

/** POST /api/v1/alert-points/{id}/update — partial update (set fields only). */
export function updateAlertPoint(id: string, patch: AlertPointPatch): Promise<AlertPoint> {
    return request(`/api/v1/alert-points/${id}/update`, {
        method: 'POST',
        body: JSON.stringify(patch)
    });
}

/** POST /api/v1/alert-points/{id}/delete — cascades probes + alerts. */
export async function deleteAlertPoint(id: string): Promise<void> {
    await request<void>(`/api/v1/alert-points/${id}/delete`, { method: 'POST' });
}

/** POST /api/v1/alert-points/{id}/test — on-demand probe (not persisted). */
export function testAlertPoint(id: string): Promise<ProbeObservation> {
    return request(`/api/v1/alert-points/${id}/test`, { method: 'POST' });
}

/** GET /api/v1/alert-points/{id}/history?hours= — uptime + slim probe series. */
export function getAlertPointHistory(id: string, hours: number): Promise<ProbeHistory> {
    return request(`/api/v1/alert-points/${id}/history?hours=${hours}`);
}

/** GET /api/v1/alert-points/{id}/incidents?hours= — failed probes with error detail. */
export function getAlertPointIncidents(id: string, hours: number): Promise<ProbeIncident[]> {
    return request(`/api/v1/alert-points/${id}/incidents?hours=${hours}`);
}

/**
 * URL of the CSV/JSON export download (`GET /{id}/history/export`).
 * The backend sets `Content-Disposition: attachment`, so assigning this URL
 * (e.g. via a hidden anchor click) triggers a file download.
 */
export function exportHistoryUrl(id: string, hours: number, format: 'csv' | 'json'): string {
    return `/api/v1/alert-points/${id}/history/export?format=${format}&hours=${hours}`;
}
