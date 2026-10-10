<script lang="ts">
    import { formatDateTime } from '$lib/format';
    import type { UserAccount, UserRole } from '$lib/types';

    let {
        users,
        busyId = null,
        currentUserId = null,
        onedit,
        ondelete
    }: {
        users: UserAccount[];
        busyId?: number | null;
        /** The logged-in admin's own id — self-delete is disabled (server also rejects). */
        currentUserId?: number | null;
        onedit: (user: UserAccount) => void;
        ondelete: (user: UserAccount) => void;
    } = $props();

    const roleBadge = (role: UserRole) =>
        role === 'admin'
            ? 'bg-sky-500/10 text-sky-600 dark:text-sky-400'
            : 'bg-surface-2 text-muted';

    // Precompute row presentation in the script (used by the template below).
    const rows = $derived(
        users.map((user) => ({
            user,
            busy: busyId === user.id,
            isSelf: user.id === currentUserId,
            roleClass: roleBadge(user.role),
            created: formatDateTime(user.created_at),
            updated: formatDateTime(user.updated_at)
        }))
    );
</script>

<div class="overflow-x-auto rounded-lg border border-line">
    <table class="w-full text-left text-sm">
        <thead class="bg-surface-2 text-xs uppercase tracking-wide text-muted">
            <tr>
                <th class="px-4 py-3">Email</th>
                <th class="px-4 py-3">Role</th>
                <th class="px-4 py-3">Created</th>
                <th class="px-4 py-3">Last updated</th>
                <th class="px-4 py-3 text-right">Actions</th>
            </tr>
        </thead>
        <tbody>
            {#each rows as row (row.user.id)}
                <tr
                    class="border-t border-line hover:bg-surface-2/50 {row.busy
                        ? 'opacity-60'
                        : ''}"
                >
                    <td class="px-4 py-3">
                        <span class="font-medium text-text">{row.user.email}</span>
                        {#if row.isSelf}
                            <span
                                class="ml-2 rounded bg-surface-2 px-1.5 py-0.5 text-xs text-muted"
                                >you</span
                            >
                        {/if}
                    </td>
                    <td class="px-4 py-3">
                        <span
                            class="rounded px-1.5 py-0.5 text-xs uppercase tracking-wide {row.roleClass}"
                        >
                            {row.user.role}
                        </span>
                    </td>
                    <td class="px-4 py-3 text-xs text-muted">{row.created}</td>
                    <td class="px-4 py-3 text-xs text-muted">{row.updated}</td>
                    <td class="px-4 py-3">
                        <div class="flex justify-end gap-1">
                            <button
                                type="button"
                                disabled={row.busy}
                                onclick={() => onedit(row.user)}
                                class="rounded-md border border-line px-2 py-1 text-xs hover:bg-surface-2 disabled:opacity-50"
                            >
                                Edit
                            </button>
                            <button
                                type="button"
                                disabled={row.busy || row.isSelf}
                                title={row.isSelf
                                    ? 'You cannot delete your own account'
                                    : 'Delete user'}
                                onclick={() => ondelete(row.user)}
                                class="rounded-md border border-line px-2 py-1 text-xs text-rose-600 hover:bg-rose-500/10 dark:text-rose-400 disabled:cursor-not-allowed disabled:opacity-50"
                            >
                                Delete
                            </button>
                        </div>
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
</div>
