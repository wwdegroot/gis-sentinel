<script lang="ts">
    import { goto } from '$app/navigation';
    import { page } from '$app/state';
    import { ApiError, UnauthorizedError } from '$lib/api';
    import { auth } from '$lib/auth.svelte';

    let email = $state('');
    let password = $state('');
    let busy = $state(false);
    let error = $state('');

    // Already logged in (e.g. revisiting /login) → go to the dashboard.
    $effect(() => {
        if (auth.ready && auth.currentUser) {
            goto(page.url.searchParams.get('redirect') ?? '/');
        }
    });

    async function handleSubmit(event: SubmitEvent) {
        event.preventDefault();
        busy = true;
        error = '';
        try {
            await auth.login(email, password);
            // Return to the page the user was bounced from, if any.
            await goto(page.url.searchParams.get('redirect') ?? '/');
        } catch (e) {
            if (e instanceof UnauthorizedError) {
                error = 'Invalid email or password.';
            } else if (e instanceof ApiError) {
                error = `${e.message} (${e.code})`;
            } else {
                error = e instanceof Error ? e.message : String(e);
            }
        } finally {
            busy = false;
        }
    }
</script>

<svelte:head>
    <title>Sign in — GIS Sentinel</title>
</svelte:head>

<div class="flex min-h-[calc(100vh-3.5rem)] items-center justify-center px-4">
    <div class="w-full max-w-sm">
        <div class="mb-6 text-center">
            <div class="text-4xl" aria-hidden="true">🛰️</div>
            <h1 class="mt-2 text-2xl font-bold">GIS Sentinel</h1>
            <p class="text-sm text-muted">Sign in to view and manage monitoring points.</p>
        </div>

        <form
            onsubmit={handleSubmit}
            class="flex flex-col gap-4 rounded-lg border border-line bg-surface p-6"
        >
            <label class="flex flex-col gap-1.5">
                <span class="text-sm font-medium">Email</span>
                <input
                    type="email"
                    bind:value={email}
                    required
                    autocomplete="username"
                    placeholder="you@example.com"
                    class="rounded-md border border-line bg-surface-2 px-3 py-2 text-sm outline-none focus:border-sky-500"
                />
            </label>

            <label class="flex flex-col gap-1.5">
                <span class="text-sm font-medium">Password</span>
                <input
                    type="password"
                    bind:value={password}
                    required
                    autocomplete="current-password"
                    placeholder="••••••••"
                    class="rounded-md border border-line bg-surface-2 px-3 py-2 text-sm outline-none focus:border-sky-500"
                />
            </label>

            {#if error}
                <div
                    class="rounded-md bg-rose-500/10 px-3 py-2 text-sm text-rose-600 dark:text-rose-400"
                    role="alert"
                >
                    {error}
                </div>
            {/if}

            <button
                type="submit"
                disabled={busy}
                class="rounded-md bg-sky-600 px-4 py-2 text-sm font-semibold text-white hover:bg-sky-500 disabled:opacity-50"
            >
                {busy ? 'Signing in…' : 'Sign in'}
            </button>
        </form>
    </div>
</div>
