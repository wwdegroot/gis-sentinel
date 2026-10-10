/**
 * Client-side validation mirroring the backend rules exactly (task 3.4 —
 * see docs/phase3.md §0.2). The backend re-validates; matching here gives
 * instant feedback and prevents avoidable 422s.
 */

import type { AlertPoint, AuthConfig, HttpMethod, NewAlertPoint, ServiceType } from './types';

/** Backend-enforced bounds (see handlers/alert_points_api.rs constants). */
export const MIN_CHECK_INTERVAL_SECS = 10;
export const MAX_CHECK_INTERVAL_SECS = 86_400;
export const MAX_EXPECTED_RESPONSE_MS = 600_000;
export const MAX_NAME_LEN = 200;

/** Headers managed by the transport; configuring them is rejected. */
const FORBIDDEN_HEADERS = new Set([
    'host',
    'content-length',
    'connection',
    'transfer-encoding',
    'upgrade',
    'te',
    'trailer',
    'proxy-connection',
    'proxy-authorization'
]);

const HTTP_TOKEN_RE = /^[a-z0-9!#$%&'*+.^_`|~-]+$/;

export function isValidHeaderName(name: string): boolean {
    if (name.length === 0) return false;
    return HTTP_TOKEN_RE.test(name.toLowerCase());
}

export function isForbiddenHeader(name: string): boolean {
    return FORBIDDEN_HEADERS.has(name.toLowerCase());
}

export function validateUrl(url: string): string | null {
    if (url.length > 2048) return 'must be at most 2048 characters';
    let parsed: URL;
    try {
        parsed = new URL(url);
    } catch {
        return 'is not a valid URL';
    }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        return 'scheme must be http or https';
    }
    if (parsed.username !== '' || parsed.password !== '') {
        return 'must not embed credentials; use auth instead';
    }
    if (!parsed.hostname) return 'is missing a host';
    return null;
}

export interface AlertPointForm {
    name: string;
    url: string;
    service_type: ServiceType;
    check_interval_seconds: number | null;
    expected_response_time_ms: number | null;
    http_method: HttpMethod;
    enabled: boolean;
    /** Key/value rows for custom headers (blank rows are dropped). */
    headers: { name: string; value: string }[];
    authKind: 'none' | 'basic' | 'bearer' | 'header';
    authUsername: string;
    authPassword: string;
    authToken: string;
    authHeaderName: string;
    authHeaderValue: string;
}

export function emptyForm(): AlertPointForm {
    return {
        name: '',
        url: '',
        service_type: 'HTTP',
        check_interval_seconds: 60,
        expected_response_time_ms: 1000,
        http_method: 'GET',
        enabled: true,
        headers: [],
        authKind: 'none',
        authUsername: '',
        authPassword: '',
        authToken: '',
        authHeaderName: '',
        authHeaderValue: ''
    };
}

export function pointToForm(point: AlertPoint): AlertPointForm {
    const form = emptyForm();
    form.name = point.name;
    form.url = point.url;
    form.service_type = point.service_type;
    form.check_interval_seconds = point.check_interval_seconds;
    form.expected_response_time_ms = point.expected_response_time_ms;
    form.http_method = point.http_method;
    form.enabled = point.enabled;
    form.headers = Object.entries(point.custom_headers ?? {}).map(([name, value]) => ({
        name,
        value
    }));

    const auth = point.auth_config;
    if (auth?.type === 'basic') {
        form.authKind = 'basic';
        form.authUsername = auth.username;
        form.authPassword = auth.password ?? '';
    } else if (auth?.type === 'bearer') {
        form.authKind = 'bearer';
        form.authToken = auth.token;
    } else if (auth?.type === 'header') {
        form.authKind = 'header';
        form.authHeaderName = auth.name;
        form.authHeaderValue = auth.value;
    }
    return form;
}

/** Build the wire payload; returns null + errors when invalid. */
export function formToPayload(
    form: AlertPointForm
): { payload: NewAlertPoint; errors: null } | { payload: null; errors: Record<string, string> } {
    const errors = validateForm(form);
    if (Object.keys(errors).length > 0) return { payload: null, errors };

    const headers: Record<string, string> = {};
    for (const row of form.headers) {
        const name = row.name.trim();
        if (name !== '') headers[name] = row.value;
    }

    let auth_config: AuthConfig | null = null;
    if (form.authKind === 'basic') {
        auth_config = {
            type: 'basic',
            username: form.authUsername.trim(),
            password: form.authPassword
        };
    } else if (form.authKind === 'bearer') {
        auth_config = { type: 'bearer', token: form.authToken };
    } else if (form.authKind === 'header') {
        auth_config = {
            type: 'header',
            name: form.authHeaderName.trim(),
            value: form.authHeaderValue
        };
    }

    return {
        payload: {
            name: form.name.trim(),
            url: form.url.trim(),
            service_type: form.service_type,
            check_interval_seconds: form.check_interval_seconds as number,
            expected_response_time_ms: form.expected_response_time_ms as number,
            http_method: form.http_method,
            custom_headers: headers,
            auth_config,
            enabled: form.enabled
        },
        errors: null
    };
}

/** Validate the whole form; returns a field → message map (empty = valid). */
export function validateForm(form: AlertPointForm): Record<string, string> {
    const errors: Record<string, string> = {};

    const name = form.name.trim();
    if (name === '') errors.name = 'must not be empty';
    else if (form.name.length > MAX_NAME_LEN)
        errors.name = `must be at most ${MAX_NAME_LEN} characters`;

    const urlError = validateUrl(form.url.trim());
    if (urlError) errors.url = urlError;

    const interval = form.check_interval_seconds;
    if (interval === null || Number.isNaN(interval)) {
        errors.check_interval_seconds = 'is required';
    } else if (interval < MIN_CHECK_INTERVAL_SECS || interval > MAX_CHECK_INTERVAL_SECS) {
        errors.check_interval_seconds = `must be between ${MIN_CHECK_INTERVAL_SECS} and ${MAX_CHECK_INTERVAL_SECS} seconds`;
    }

    const expected = form.expected_response_time_ms;
    if (expected === null || Number.isNaN(expected)) {
        errors.expected_response_time_ms = 'is required';
    } else if (expected < 1 || expected > MAX_EXPECTED_RESPONSE_MS) {
        errors.expected_response_time_ms = `must be between 1 and ${MAX_EXPECTED_RESPONSE_MS} ms`;
    }

    for (const row of form.headers) {
        const name = row.name.trim();
        const value = row.value.trim();
        // Fully blank rows are ignored entirely (they get dropped on save).
        if (name === '' && value === '') continue;
        if (name === '') {
            errors.headers = 'header name must not be empty';
            continue;
        }
        if (!isValidHeaderName(name)) {
            errors.headers = `'${name}' is not a valid HTTP header name`;
            break;
        }
        if (isForbiddenHeader(name)) {
            errors.headers = `'${name}' is managed by the server and cannot be overridden`;
            break;
        }
    }

    if (form.authKind === 'basic' && form.authUsername.trim() === '') {
        errors.auth = "type 'basic' requires a username";
    } else if (form.authKind === 'bearer' && form.authToken === '') {
        errors.auth = "type 'bearer' requires a token";
    } else if (form.authKind === 'header') {
        if (!isValidHeaderName(form.authHeaderName.trim())) {
            errors.auth = 'requires a valid header name';
        } else if (isForbiddenHeader(form.authHeaderName.trim())) {
            errors.auth = 'that header is managed by the server';
        }
    }

    return errors;
}
