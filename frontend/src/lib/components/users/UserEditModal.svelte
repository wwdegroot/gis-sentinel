<script lang="ts">
    import { updateUser, ApiError } from '$lib/api';
    import type { UserAccount, UserRole } from '$lib/types';

    let {
        user,
        currentUserId = null,
        onSaved,
        onClose
    }: {
        user: UserAccount;
        /** The logged-in admin's own id — self-demote is disabled (server also rejects). */
        currentUserId?: number | null;
        onSaved: (user: UserAccount) => void;
        onClose: () => void;
    } = $props();

    const roles: UserRole[] = ['admin', 'viewer'];

    // Snapshot the target once — the modal is remounted for every open.
    // svelte-ignore state_referenced_locally
    const initial: UserAccount = $state.snapshot(user);

    let email = $state(initial.email);
    let role = $state<UserRole>(initial.role);
    // Optional password rotation: empty string = leave unchanged.
    let password = $state('');
    let errors = $state<Record<string, string>>({});
    let saving = $state(false);
    let serverError = $state('');

    const isSelf = $derived(initial.id === currentUserId);

    function validate(): boolean {
        const next: Record<string, string> = {};
        if (!email.trim()) next.email = 'Email is required';
        else if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim()))
            next.email = 'Enter a valid email address';
        if (password && password.length < 8) next.password = 'At least 8 characters';
        errors = next;
        return Object.keys(next).length === 0;
    }

    async function save(event: SubmitEvent) {
        event.preventDefault();
        serverError = '';
        if (!validate()) return;
        saving = true;
        const patch: { email?: string; role?: UserRole; password?: string } = {};
        if (email.trim() !== initial.email) patch.email = email.trim();
        if (role !== initial.role) patch.role = role;
        if (password) patch.password = password;
        try {
            // Nothing changed → treat as a no-op close (avoids a needless
            // session revocation of the edited user).
            const updated =
                Object.keys(patch).length > 0 ? await updateUser(initial.id, patch) : initial;
            onSaved(updated);
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
            <h2 class="text-lg font-bold">Edit “{initial.email}”</h2>
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
                    class="rounded-md border border-line bg-surface px-3 py-2 {errors.email
                        ? 'border-rose-500'
                        : ''}"
                />
                {#if errors.email}<span class="text-xs text-rose-500">{errors.email}</span>{/if}
            </label>

            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Role</span>
                <select
                    bind:value={role}
                    disabled={isSelf}
                    class="rounded-md border border-line bg-surface px-3 py-2 disabled:opacity-60"
                >
                    {#each roles as r (r)}
                        <option value={r}>{r === 'admin' ? 'Admin — full access' : 'Viewer — read-only'}</option>
                    {/each}
                </select>
                {#if isSelf}
                    <span class="text-xs text-muted">
                        You cannot change your own role (the last admin could lock themselves out).
                    </span>
                {/if}
            </label>

            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Reset password</span>
                <input
                    type="password"
                    bind:value={password}
                    oninput={() => errors.password && validate()}
                    autocomplete="new-password"
                    placeholder="Leave empty to keep the current password"
                    class="rounded-md border border-line bg-surface px-3 py-2 {errors.password
                        ? 'border-rose-500'
                        : ''}"
                />
                {#if errors.password}<span class="text-xs text-rose-500">{errors.password}</span>{:else}
                    <span class="text-xs text-muted">
                        Setting a new password signs this user out everywhere.
                    </span>
                {/if}
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
                    {saving ? 'Saving…' : 'Save changes'}
                </button>
            </div>
        </form>
    </div>
</div>
