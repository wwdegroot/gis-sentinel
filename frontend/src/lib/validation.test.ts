import { describe, it, expect } from 'vitest';
import {
    emptyForm,
    formToPayload,
    validateForm,
    validateUrl,
    isValidHeaderName,
    isForbiddenHeader
} from './validation';

function validForm() {
    const form = emptyForm();
    form.name = 'GeoServer Demo';
    form.url = 'https://example.com/wms';
    form.check_interval_seconds = 30;
    form.expected_response_time_ms = 500;
    return form;
}

describe('validateUrl', () => {
    it('accepts http/https URLs without credentials', () => {
        expect(validateUrl('https://example.com/wms')).toBeNull();
        expect(validateUrl('http://localhost:8080/geoserver')).toBeNull();
    });

    it('rejects bad input', () => {
        expect(validateUrl('not a url')).toMatch(/not a valid URL/);
        expect(validateUrl('ftp://example.com')).toMatch(/scheme/);
        expect(validateUrl('https://user:pass@example.com')).toMatch(/credentials/);
        expect(validateUrl('')).toMatch(/not a valid URL/);
    });
});

describe('header name checks', () => {
    it('accepts valid header names (case-insensitive)', () => {
        expect(isValidHeaderName('X-Api-Key')).toBe(true);
        expect(isValidHeaderName('accept')).toBe(true);
    });

    it('rejects invalid header names', () => {
        expect(isValidHeaderName('X Key')).toBe(false);
        expect(isValidHeaderName('')).toBe(false);
    });

    it('flags hop-by-hop headers', () => {
        expect(isForbiddenHeader('Host')).toBe(true);
        expect(isForbiddenHeader('connection')).toBe(true);
        expect(isForbiddenHeader('X-Api-Key')).toBe(false);
    });
});

describe('validateForm', () => {
    it('accepts a complete valid form', () => {
        expect(validateForm(validForm())).toEqual({});
    });

    it('enforces backend bounds', () => {
        const form = validForm();
        form.name = '';
        form.check_interval_seconds = 5;
        form.expected_response_time_ms = 0;
        const errors = validateForm(form);
        expect(errors.name).toBeDefined();
        expect(errors.check_interval_seconds).toMatch(/between 10 and 86400/);
        expect(errors.expected_response_time_ms).toMatch(/between 1 and 600000/);
    });

    it('rejects forbidden and malformed custom headers', () => {
        const form = validForm();
        form.headers = [{ name: 'Host', value: 'evil.com' }];
        expect(validateForm(form).headers).toMatch(/managed by the server/);

        form.headers = [{ name: 'X Key', value: 'v' }];
        expect(validateForm(form).headers).toMatch(/not a valid HTTP header name/);
    });

    it('validates each auth shape', () => {
        const basic = validForm();
        basic.authKind = 'basic';
        basic.authUsername = '';
        expect(validateForm(basic).auth).toMatch(/requires a username/);

        const bearer = validForm();
        bearer.authKind = 'bearer';
        bearer.authToken = '';
        expect(validateForm(bearer).auth).toMatch(/requires a token/);

        const header = validForm();
        header.authKind = 'header';
        header.authHeaderName = 'X Key';
        header.authHeaderValue = 'v';
        expect(validateForm(header).auth).toMatch(/valid header name/);

        header.authHeaderName = 'X-Key';
        expect(validateForm(header).auth).toBeUndefined();
    });
});

describe('formToPayload', () => {
    it('builds the wire payload and drops blank header rows', () => {
        const form = validForm();
        form.headers = [
            { name: 'X-Api-Key', value: 'secret' },
            { name: '', value: '' }
        ];
        const { payload, errors } = formToPayload(form);
        expect(errors).toBeNull();
        expect(payload).toMatchObject({
            name: 'GeoServer Demo',
            service_type: 'HTTP',
            check_interval_seconds: 30,
            expected_response_time_ms: 500,
            custom_headers: { 'X-Api-Key': 'secret' },
            auth_config: null,
            enabled: true
        });
    });

    it('maps each auth kind to the backend auth_config shape', () => {
        const basic = validForm();
        basic.authKind = 'basic';
        basic.authUsername = 'u';
        basic.authPassword = 'p';
        expect(formToPayload(basic).payload?.auth_config).toEqual({
            type: 'basic',
            username: 'u',
            password: 'p'
        });

        const bearer = validForm();
        bearer.authKind = 'bearer';
        bearer.authToken = 'tok';
        expect(formToPayload(bearer).payload?.auth_config).toEqual({
            type: 'bearer',
            token: 'tok'
        });

        const header = validForm();
        header.authKind = 'header';
        header.authHeaderName = 'X-Key';
        header.authHeaderValue = 'v';
        expect(formToPayload(header).payload?.auth_config).toEqual({
            type: 'header',
            name: 'X-Key',
            value: 'v'
        });
    });

    it('returns errors instead of a payload for invalid forms', () => {
        const form = validForm();
        form.url = 'ftp://x';
        const { payload, errors } = formToPayload(form);
        expect(payload).toBeNull();
        expect(errors?.url).toBeDefined();
    });
});
