import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { SentinelSocket, computeBackoffDelay, resolveWebSocketUrl } from './sentinelSocket.svelte';
import type { AlertEvent } from './types';

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

function alert(overrides: Partial<AlertEvent> = {}): AlertEvent {
    return {
        alert_id: 1,
        alert_point_id: '01a0b500-0000-0000-0000-000000000000',
        alert_type: 'New',
        name: 'GeoServer Demo',
        url: 'https://example.com/wms',
        service_type: 'WMS',
        status: 'down',
        reason: 'connection failed',
        response_time_ms: null,
        expected_response_time_ms: 500,
        triggered_at: '2026-09-18T15:43:39.773Z',
        ...overrides
    };
}

function msgEvent(data: unknown): { data: unknown } {
    return { data: typeof data === 'string' ? data : JSON.stringify(data) };
}

let warnSpy: ReturnType<typeof vi.spyOn>;
let errorSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

// ---------------------------------------------------------------------------
// message handling (the core state machine)
// ---------------------------------------------------------------------------

describe('SentinelSocket.processMessage', () => {
    it('replaces the whole list on a snapshot', () => {
        const socket = new SentinelSocket();
        socket.processMessage(
            msgEvent({ type: 'snapshot', alerts: [alert({ alert_id: 1 }), alert({ alert_id: 2 })] })
        );
        expect(socket.alerts.length).toBe(2);
        expect(socket.alerts.map((a) => a.alert_id)).toEqual([1, 2]);

        // a second snapshot fully replaces (incl. removals)
        socket.processMessage(msgEvent({ type: 'snapshot', alerts: [alert({ alert_id: 7 })] }));
        expect(socket.alerts.map((a) => a.alert_id)).toEqual([7]);
    });

    it('adds New alerts and is defensive against duplicate New', () => {
        const socket = new SentinelSocket();
        socket.processMessage(msgEvent({ type: 'alert', ...alert({ alert_id: 5 }) }));
        socket.processMessage(msgEvent({ type: 'alert', ...alert({ alert_id: 6 }) }));
        expect(socket.alerts.length).toBe(2);

        // duplicate New for a held id replaces instead of appending
        const dup = alert({ alert_id: 5, status: 'degraded' });
        socket.processMessage(msgEvent({ type: 'alert', ...dup }));
        expect(socket.alerts.length).toBe(2);
        expect(socket.alerts.find((a) => a.alert_id === 5)?.status).toBe('degraded');
    });

    it('applies Update to the matching alert_id and ignores unknown ids', () => {
        const socket = new SentinelSocket();
        socket.processMessage(msgEvent({ type: 'alert', ...alert({ alert_id: 5 }) }));

        const update = alert({
            alert_id: 5,
            alert_type: 'Update',
            status: 'degraded',
            reason: 'latency 900 ms'
        });
        socket.processMessage(msgEvent({ type: 'alert', ...update }));
        expect(socket.alerts.length).toBe(1);
        expect(socket.alerts[0].status).toBe('degraded');
        expect(socket.alerts[0].alert_type).toBe('Update');

        socket.processMessage(
            msgEvent({ type: 'alert', ...alert({ alert_id: 99, alert_type: 'Update' }) })
        );
        expect(socket.alerts.length).toBe(1);
        expect(warnSpy).toHaveBeenCalled();
    });

    it('removes the matching alert_id on Remove', () => {
        const socket = new SentinelSocket();
        socket.processMessage(msgEvent({ type: 'alert', ...alert({ alert_id: 5 }) }));
        socket.processMessage(msgEvent({ type: 'alert', ...alert({ alert_id: 6 }) }));

        socket.processMessage(
            msgEvent({
                type: 'alert',
                ...alert({ alert_id: 5, alert_type: 'Remove', status: 'healthy' })
            })
        );
        expect(socket.alerts.map((a) => a.alert_id)).toEqual([6]);
    });

    it('swallows invalid JSON and malformed messages without throwing', () => {
        const socket = new SentinelSocket();
        expect(() => socket.processMessage(msgEvent('not json at all'))).not.toThrow();
        expect(() => socket.processMessage(msgEvent({ foo: 'bar' }))).not.toThrow();
        expect(() => socket.processMessage(msgEvent(42))).not.toThrow();
        expect(socket.alerts).toEqual([]);
        expect(errorSpy).toHaveBeenCalled();
    });
});

// ---------------------------------------------------------------------------
// backoff computation
// ---------------------------------------------------------------------------

describe('computeBackoffDelay', () => {
    it('doubles from the base delay', () => {
        expect(computeBackoffDelay(1)).toBe(500);
        expect(computeBackoffDelay(2)).toBe(1000);
        expect(computeBackoffDelay(3)).toBe(2000);
        expect(computeBackoffDelay(4)).toBe(4000);
    });

    it('caps at 30 seconds', () => {
        expect(computeBackoffDelay(7)).toBe(30_000);
        expect(computeBackoffDelay(50)).toBe(30_000);
    });
});

// ---------------------------------------------------------------------------
// reconnect scheduling (fake WebSocket + fake timers)
// ---------------------------------------------------------------------------

class FakeWebSocket {
    static instances: FakeWebSocket[] = [];
    static OPEN = 1;
    static CONNECTING = 0;
    static CLOSED = 3;

    readyState = FakeWebSocket.CONNECTING;
    listeners = new Map<string, ((ev?: unknown) => void)[]>();

    constructor(public url: string) {
        FakeWebSocket.instances.push(this);
    }

    addEventListener(type: string, cb: (ev?: unknown) => void) {
        this.listeners.set(type, [...(this.listeners.get(type) ?? []), cb]);
    }

    close() {
        this.readyState = FakeWebSocket.CLOSED;
        this.emit('close');
    }

    emit(type: string, ev?: unknown) {
        if (type === 'open') this.readyState = FakeWebSocket.OPEN;
        this.listeners.get(type)?.forEach((cb) => cb(ev));
    }
}

function stubWebSocket() {
    FakeWebSocket.instances = [];
    vi.stubGlobal('WebSocket', FakeWebSocket);
}

describe('reconnect scheduling', () => {
    it('reconnects with growing delays and resets the counter on success', () => {
        vi.useFakeTimers();
        stubWebSocket();
        const socket = new SentinelSocket('ws://test/ws/sentinel');

        socket.connect();
        expect(FakeWebSocket.instances.length).toBe(1);
        expect(socket.connection).toBe('connecting');

        // successful open
        FakeWebSocket.instances[0].emit('open');
        expect(socket.connection).toBe('connected');
        expect(socket.retryCount).toBe(0);

        // drop -> schedules reconnect #1 after ~500ms
        FakeWebSocket.instances[0].emit('close');
        expect(socket.connection).toBe('disconnected');
        expect(socket.retryCount).toBe(1);

        vi.advanceTimersByTime(computeBackoffDelay(1) + 250);
        expect(FakeWebSocket.instances.length).toBe(2);
        expect(socket.connection).toBe('connecting');

        // drop again -> reconnect #2 after ~1000ms
        FakeWebSocket.instances[1].emit('close');
        expect(socket.retryCount).toBe(2);
        vi.advanceTimersByTime(computeBackoffDelay(1) + 250);
        expect(FakeWebSocket.instances.length).toBe(2); // too early
        vi.advanceTimersByTime(computeBackoffDelay(2));
        expect(FakeWebSocket.instances.length).toBe(3);

        // open resets the backoff counter
        FakeWebSocket.instances[2].emit('open');
        expect(socket.retryCount).toBe(0);
        expect(socket.connection).toBe('connected');
    });

    it('does not reconnect after a user-initiated close', () => {
        vi.useFakeTimers();
        stubWebSocket();
        const socket = new SentinelSocket('ws://test/ws/sentinel');

        socket.connect();
        FakeWebSocket.instances[0].emit('open');
        socket.close(1000);

        expect(socket.connection).toBe('disconnected');
        vi.advanceTimersByTime(60_000);
        expect(FakeWebSocket.instances.length).toBe(1); // no new connection
    });

    it('stale sockets (superseded reconnects) do not clobber live state', () => {
        vi.useFakeTimers();
        stubWebSocket();
        const socket = new SentinelSocket('ws://test/ws/sentinel');

        socket.connect();
        const first = FakeWebSocket.instances[0];
        first.emit('close'); // schedules reconnect
        vi.advanceTimersByTime(computeBackoffDelay(1) + 250);
        const second = FakeWebSocket.instances[1];
        second.emit('open');
        expect(socket.connection).toBe('connected');

        // late close of the stale first socket must be ignored
        first.emit('close');
        expect(socket.connection).toBe('connected');
    });
});

// ---------------------------------------------------------------------------
// URL resolution
// ---------------------------------------------------------------------------

describe('resolveWebSocketUrl', () => {
    it('uses ws:// on http pages', () => {
        vi.stubGlobal('location', { protocol: 'http:', host: 'localhost:5173' });
        expect(resolveWebSocketUrl()).toBe('ws://localhost:5173/ws/sentinel');
    });

    it('uses wss:// on https pages', () => {
        vi.stubGlobal('location', { protocol: 'https:', host: 'sentinel.example.com' });
        expect(resolveWebSocketUrl()).toBe('wss://sentinel.example.com/ws/sentinel');
    });
});
