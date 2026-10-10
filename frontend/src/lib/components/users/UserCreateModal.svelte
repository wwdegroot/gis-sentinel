<script lang="ts">
    import { createUser, ApiError } from '$lib/api';
    import type { UserAccount, UserRole } from '$lib/types';

    let { onSaved, onClose }: { onSaved: (user: UserAccount) => void; onClose: () => void } =
        $props();

    const roles: UserRole[] = ['admin', 'viewer'];

    let email = $state('');
    let password = $state('');
    let role = $state<UserRole>('viewer');
    let errors = $state<Record<string, string>>({});
    let saving = $state(false);
    let serverError = $state('');

    function validate(): boolean {
        const next: Record<string, string> = {};
        if (!email.trim()) next.email = 'Email is required';
        else if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim()))
            next.email = 'Enter a valid email address';
        if (password.length < 8) next.password = 'At least 8 characters';
        errors = next;
        return Object.keys(next).length === 0;
    }

    async function save(event: SubmitEvent) {
        event.preventDefault();
        serverError = '';
        if (!validate()) return;
        saving = true;
        try {
            const created = await createUser({
                email: email.trim(),
                password,
                role
            });
            onSaved(created);
        } catch (e) {
            if (e instanceof ApiError) {
                serverError = e.message;
                if (e.details) errors = { ...errors, ...e.details };
            } else {
                serverError = e instanceof Error ? e.message : String(e);
            }
        } finally {
            saving = false;
        }
    }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog" aria-modal="true">
    <div class="w-full max-w-md rounded-xl border border-line bg-surface p-6 shadow-xl">
        <div class="mb-4 flex items-center justify-between">
            <h2 class="text-lg font-bold">New user</h2>
            <button
                type="button"
                onclick={onClose}
                aria-label="Close"
                class="rounded-md p-1 text-muted hover:bg-surface-2 hover:text-text"
            >
                ✕
            </button>
        </div>

        <form class="flex flex-col gap-4" onsubmit={save}>
            {#if serverError}
                <div
                    class="rounded-md bg-rose-500/10 px-3 py-2 text-sm text-rose-600 dark:text-rose-400"
                >
                    {serverError}
                </div>
            {/if}

            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Email</span>
                <input
                    type="email"
                    bind:value={email}
                    oninput={() => errors.email && validate()}
                    placeholder="user@example.com"
                    class="rounded-md border border-line bg-surface px-3 py-2 {errors.email
                        ? 'border-rose-500'
                        : ''}"
                />
                {#if errors.email}<span class="text-xs text-rose-500">{errors.email}</span>{/if}
            </label>

            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Password</span>
                <input
                    type="password"
                    bind:value={password}
                    oninput={() => errors.password && validate()}
                    autocomplete="new-password"
                    class="rounded-md border border-line bg-surface px-3 py-2 {errors.password
                        ? 'border-rose-500'
                        : ''}"
                />
                <span class="text-xs text-muted">At least 8 characters.</span>
                {#if errors.password}<span class="text-xs text-rose-500">{errors.password}</span>{/if}
            </label>

            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Role</span>
                <select
                    bind:value={role}
                    class="rounded-md border border-line bg-surface px-3 py-2"
                >
                    {#each roles as r (r)}
                        <option value={r}>{r === 'admin' ? 'Admin — full access' : 'Viewer — read-only'}</option>
                    {/each}
                </select>
            </label>

            <div class="mt-2 flex justify-end gap-2">
                <button
                    type="button"
                    onclick={onClose}
                    class="rounded-md border border-line px-4 py-2 text-sm hover:bg-surface-2"
                >
                    Cancel
                </button>
                <button
                    type="submit"
                    disabled={saving}
                    class="rounded-md bg-sky-600 px-4 py-2 text-sm font-semibold text-white hover:bg-sky-500 disabled:opacity-50"
                >
                    {saving ? 'Creating…' : 'Create user'}
                </button>
            </div>
        </form>
    </div>
</div>
