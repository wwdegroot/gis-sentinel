/**
 * Auth store (auth plan §9): session-cookie auth state for the SPA.
 *
 * - `init()` runs once on app load (layout `onMount`) — `GET /auth/me`
 *   resolves the session cookie into `currentUser`, or leaves it `null`.
 * - `login()`/`logout()` call the backend and update the socket: the WS
 *   upgrade is cookie-authenticated, so it connects after login and is
 *   user-closed (reconnect loop stopped) on logout.
 * - A global unauthorized handler (installed via `api.setUnauthorizedHandler`)
 *   redirects to `/login` whenever any API call 401s mid-session — except on
 *   the login page itself, so failed logins don't loop.
 */

import { goto } from '$app/navigation';
import { login, logout, me, setUnauthorizedHandler } from './api';
import type { User } from './types';

export class AuthStore {
    /** The logged-in user, or `null` when logged out / not yet resolved. */
    currentUser: User | null = $state(null);
    /** True once the initial `me()` check has completed. */
    ready: boolean = $state(false);

    /** True when the current user may use write endpoints (POST routes). */
    get isAdmin(): boolean {
        return this.currentUser?.role === 'admin';
    }

    async init(): Promise<void> {
        try {
            this.currentUser = await me();
        } catch {
            // 401 (or network error) — treat as logged out; the redirect to
            // /login is driven by the layout's $effect on `currentUser`.
            this.currentUser = null;
        } finally {
            this.ready = true;
        }
    }

    /** Login with email+password; connects the alert WebSocket on success. */
    async login(email: string, password: string): Promise<void> {
        this.currentUser = await login({ email, password });
    }

    /** Logout: stop the (now-unauthorized) WS reconnect loop, clear the session. */
    async logout(): Promise<void> {
        try {
            await logout();
        } catch {
            // Best-effort: clear local state even if the backend is unreachable.
        } finally {
            this.currentUser = null;
        }
    }
}

export const auth = new AuthStore();

// Redirect to /login on any mid-session 401. On the login page itself this is
// a no-op so an invalid-credentials 401 from `POST /auth/login` doesn't loop.
setUnauthorizedHandler(() => {
    if (auth.currentUser) {
        auth.currentUser = null;
    }
    if (typeof location !== 'undefined' && !location.pathname.startsWith('/login')) {
        goto('/login');
    }
});
