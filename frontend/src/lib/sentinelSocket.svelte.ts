import type { AlertEvent, ConnectionState, WsMessage } from './types';

/** First reconnect waits this long (exponential growth afterwards). */
const BASE_DELAY_MS = 500;
/** Reconnect delay cap. */
const MAX_DELAY_MS = 30_000;
/** Max number of alert cards kept in memory (guard against runaway state). */
const MAX_ALERTS = 500;

/**
 * Resolve the sentinel WebSocket URL for the current origin:
 * same-origin host with `wss:` on HTTPS pages, `ws:` otherwise.
 * In `vite dev` the `/ws` proxy forwards to the backend.
 */
export function resolveWebSocketUrl(): string {
    if (typeof location === 'undefined') {
        // SSR/no-browser safety; the app runs with `ssr = false`.
        return '';
    }
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${protocol}//${location.host}/ws/sentinel`;
}

/**
 * Deterministic part of the reconnect backoff (jitter is added separately):
 * 500 ms, 1 s, 2 s, … capped at 30 s.
 */
export function computeBackoffDelay(retryCount: number): number {
    if (retryCount <= 1) return BASE_DELAY_MS;
    const delay = BASE_DELAY_MS * 2 ** (retryCount - 1);
    return Math.min(delay, MAX_DELAY_MS);
}

/**
 * Live-alert store backed by the `/ws/sentinel` WebSocket (task 3.1).
 *
 * Message contract (verified in Phase 2 — see docs/phase2.md §5):
 * - first message is `{"type":"snapshot","alerts":[...]}` — replaces state;
 * - live events are `{"type":"alert", ...event}` (fields flattened) with
 *   `alert_type` of `New`/`Update`/`Remove`, keyed by `alert_id`.
 *
 * Reconnects automatically with exponential backoff + jitter; a fresh
 * snapshot on reconnect is the re-sync mechanism.
 *
 * Exported as a singleton (`sentinel`) so the nav connection badge and the
 * dashboard share one connection.
 */
export class SentinelSocket {
    socket?: WebSocket;
    /** Open alerts, keyed by `alert_id` (latest event per id wins). */
    alerts: AlertEvent[] = $state<AlertEvent[]>([]);
    connection: ConnectionState = $state<ConnectionState>('disconnected');
    /** Number of reconnect attempts since the last successful open. */
    retryCount: number = $state(0);
    address: string = '';

    private reconnectTimer?: ReturnType<typeof setTimeout>;
    private closedByUser = false;

    constructor(address: string = resolveWebSocketUrl()) {
        this.address = address;
    }

    /** True while a reconnect is scheduled/waiting (badge: 🟡). */
    get reconnecting(): boolean {
        return !this.closedByUser && this.connection !== 'connected' && this.retryCount > 0;
    }

    /**
     * Open the connection (idempotent while open/connecting). Cancels any
     * pending reconnect and restarts the backoff sequence.
     */
    connect(): void {
        this.cancelReconnectTimer();
        if (this.isOpenOrConnecting()) return;
        this.closedByUser = false;
        if (!this.address) this.address = resolveWebSocketUrl();
        this.connection = 'connecting';
        this.open();
    }

    /** User-initiated close: stops the reconnect loop permanently. */
    close(code: number = 1000, reason?: string): void {
        this.closedByUser = true;
        this.cancelReconnectTimer();
        this.retryCount = 0;
        this.connection = 'disconnected';
        this.socket?.close(code, reason);
        this.socket = undefined;
    }

    /**
     * Route one incoming message into local state. Malformed messages are
     * logged and swallowed — the listener must never throw.
     */
    processMessage(event: { data: unknown }): void {
        let message: unknown;
        try {
            message = JSON.parse(String(event.data));
        } catch (e) {
            console.error('sentinel socket: received invalid JSON', e);
            return;
        }
        this.handleMessage(message);
    }

    // ------------------------------------------------------------------
    // internals
    // ------------------------------------------------------------------

    private handleMessage(message: unknown): void {
        if (!message || typeof message !== 'object' || !('type' in message)) {
            console.warn('sentinel socket: ignoring malformed message', message);
            return;
        }
        const msg = message as WsMessage;

        switch (msg.type) {
            case 'snapshot': {
                // Full replace — also the re-sync path after a reconnect.
                this.alerts = msg.alerts ?? [];
                break;
            }
            case 'alert': {
                const alert: AlertEvent = msg;
                switch (alert.alert_type) {
                    case 'New': {
                        const idx = this.findIndex(alert.alert_id);
                        if (idx !== -1) {
                            // Defensive: duplicate New for an id we already hold.
                            this.alerts[idx] = alert;
                        } else {
                            this.alerts.push(alert);
                            if (this.alerts.length > MAX_ALERTS) {
                                this.alerts = this.alerts.slice(-MAX_ALERTS);
                            }
                        }
                        break;
                    }
                    case 'Update': {
                        const idx = this.findIndex(alert.alert_id);
                        if (idx !== -1) {
                            this.alerts[idx] = alert;
                        } else {
                            console.warn(
                                'sentinel socket: update for unknown alert_id',
                                alert.alert_id
                            );
                        }
                        break;
                    }
                    case 'Remove': {
                        this.alerts = this.alerts.filter((a) => a.alert_id !== alert.alert_id);
                        break;
                    }
                    default: {
                        console.warn('sentinel socket: unknown alert_type', alert.alert_type);
                    }
                }
                break;
            }
            default: {
                console.warn(
                    'sentinel socket: unknown message type',
                    (msg as { type?: string }).type
                );
            }
        }
    }

    private findIndex(alertId: number): number {
        return this.alerts.findIndex((a) => a.alert_id === alertId);
    }

    private isOpenOrConnecting(): boolean {
        const state = this.socket?.readyState;
        return state === WebSocket.OPEN || state === WebSocket.CONNECTING;
    }

    private open(): void {
        const socket = new WebSocket(this.address);
        this.socket = socket;

        socket.addEventListener('open', () => {
            if (this.socket !== socket) return; // stale socket
            this.retryCount = 0;
            this.connection = 'connected';
        });

        socket.addEventListener('message', (event) => this.processMessage(event));

        socket.addEventListener('close', () => {
            if (this.socket !== socket) return; // stale socket
            this.socket = undefined;
            this.connection = 'disconnected';
            this.scheduleReconnect();
        });

        socket.addEventListener('error', () => {
            // The close event always follows an error; state + reconnect are
            // handled there. Never throw from a listener.
            if (this.socket === socket && this.connection === 'connecting') {
                this.connection = 'disconnected';
            }
            console.error('sentinel socket: connection error');
        });
    }

    private scheduleReconnect(): void {
        if (this.closedByUser) return;
        this.retryCount += 1;
        const jitter = Math.floor(Math.random() * 250);
        const delay = computeBackoffDelay(this.retryCount) + jitter;
        this.reconnectTimer = setTimeout(() => {
            this.reconnectTimer = undefined;
            if (this.closedByUser) return;
            this.connection = 'connecting';
            this.open();
        }, delay);
    }

    private cancelReconnectTimer(): void {
        if (this.reconnectTimer !== undefined) {
            clearTimeout(this.reconnectTimer);
            this.reconnectTimer = undefined;
        }
    }
}

/** App-wide singleton (see class doc for why). */
export const sentinel = new SentinelSocket();
