<script lang="ts">
    import { changePassword, ApiError } from '$lib/api';
    import { auth } from '$lib/auth.svelte';
    import { theme } from '$lib/theme.svelte';

    // Password change (self-service): verify current password, then rotate.
    // The backend signs out every *other* device and keeps this session alive.
    let currentPassword = $state('');
    let newPassword = $state('');
    let confirmPassword = $state('');
    let changing = $state(false);
    let changeError = $state('');
    let changeSuccess = $state(false);

    function validateChange(): string {
        if (!currentPassword) return 'Enter your current password.';
        if (newPassword.length < 8) return 'New password must be at least 8 characters.';
        if (newPassword !== confirmPassword) return 'New passwords do not match.';
        if (newPassword === currentPassword)
            return 'The new password must differ from the current one.';
        return '';
    }

    async function handleChange(event: SubmitEvent) {
        event.preventDefault();
        changeError = '';
        changeSuccess = false;
        const problem = validateChange();
        if (problem) {
            changeError = problem;
            return;
        }
        changing = true;
        try {
            await changePassword({
                current_password: currentPassword,
                new_password: newPassword
            });
            currentPassword = '';
            newPassword = '';
            confirmPassword = '';
            changeSuccess = true;
        } catch (e) {
            changeError =
                e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
        } finally {
            changing = false;
        }
    }
</script>

<div class="mx-auto max-w-xl">
    <h1 class="text-xl font-bold">Settings</h1>
    <p class="mt-1 text-sm text-muted">Global thresholds and notifications (planned).</p>

    <!-- Account (self-service credentials) -->
    <div class="mt-6 rounded-lg border border-line p-4">
        <div class="font-medium">Account</div>
        <div class="mt-0.5 text-sm text-muted">
            Signed in as <span class="font-mono">{auth.currentUser?.email ?? '—'}</span>
            <span class="ml-1 rounded bg-surface-2 px-1.5 py-0.5 text-xs uppercase tracking-wide">
                {auth.currentUser?.role ?? '—'}
            </span>
        </div>

        <form class="mt-4 flex flex-col gap-3" onsubmit={handleChange}>
            <div class="text-sm font-medium">Change password</div>

            {#if changeError}
                <div
                    class="rounded-md bg-rose-500/10 px-3 py-2 text-sm text-rose-600 dark:text-rose-400"
                >
                    {changeError}
                </div>
            {:else if changeSuccess}
                <div
                    class="rounded-md bg-emerald-500/10 px-3 py-2 text-sm text-emerald-600 dark:text-emerald-400"
                >
                    Password updated. Your other sessions were signed out.
                </div>
            {/if}

            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Current password</span>
                <input
                    type="password"
                    bind:value={currentPassword}
                    autocomplete="current-password"
                    class="rounded-md border border-line bg-surface px-3 py-2"
                />
            </label>

            <div class="grid gap-3 sm:grid-cols-2">
                <label class="flex flex-col gap-1 text-sm">
                    <span class="font-medium">New password</span>
                    <input
                        type="password"
                        bind:value={newPassword}
                        autocomplete="new-password"
                        class="rounded-md border border-line bg-surface px-3 py-2"
                    />
                </label>
                <label class="flex flex-col gap-1 text-sm">
                    <span class="font-medium">Repeat new password</span>
                    <input
                        type="password"
                        bind:value={confirmPassword}
                        autocomplete="new-password"
                        class="rounded-md border border-line bg-surface px-3 py-2"
                    />
                </label>
            </div>
            <div class="text-xs text-muted">At least 8 characters.</div>

            <div class="flex justify-end">
                <button
                    type="submit"
                    disabled={changing}
                    class="rounded-md bg-sky-600 px-4 py-2 text-sm font-semibold text-white hover:bg-sky-500 disabled:opacity-50"
                >
                    {changing ? 'Updating…' : 'Update password'}
                </button>
            </div>
        </form>
    </div>

    <!-- Theme -->
    <div class="mt-4 flex items-center justify-between rounded-lg border border-line p-4">
        <div>
            <div class="font-medium">Theme</div>
            <div class="text-sm text-muted">Currently: {theme.current}</div>
        </div>
        <button
            type="button"
            onclick={() => theme.toggle()}
            class="rounded-md bg-surface-2 px-3 py-1.5 text-sm font-medium hover:opacity-80"
        >
            Switch to {theme.current === 'dark' ? 'light' : 'dark'}
        </button>
    </div>
</div>
