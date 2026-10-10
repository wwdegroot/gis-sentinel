<script lang="ts">
    import { onMount } from 'svelte';
    import { goto } from '$app/navigation';
    import { deleteUser, listUsers, ApiError } from '$lib/api';
    import { auth } from '$lib/auth.svelte';
    import type { UserAccount } from '$lib/types';
    import UsersTable from '$lib/components/users/UsersTable.svelte';
    import UserCreateModal from '$lib/components/users/UserCreateModal.svelte';
    import UserEditModal from '$lib/components/users/UserEditModal.svelte';
    import UserDeleteConfirm from '$lib/components/users/UserDeleteConfirm.svelte';

    // Admin-only page (plan Q2-a): viewers/unauthenticated never render it —
    // the layout hides the nav link and the server 403s the endpoints anyway.
    let adminChecked = $state(false);
    $effect(() => {
        if (auth.ready && !auth.isAdmin) {
            goto('/');
        } else if (auth.ready) {
            adminChecked = true;
        }
    });

    let users = $state<UserAccount[]>([]);
    let loading = $state(true);
    let loadError = $state('');

    // modal / dialog state
    let createOpen = $state(false);
    let editing = $state<UserAccount | null>(null);
    let deleting = $state<UserAccount | null>(null);
    let deleteBusy = $state(false);

    // in-flight mutation (row-level busy indicator)
    let busyId = $state<number | null>(null);
    let actionError = $state('');

    async function load() {
        loading = true;
        loadError = '';
        try {
            users = await listUsers();
        } catch (e) {
            loadError = describe(e);
        } finally {
            loading = false;
        }
    }

    onMount(() => {
        if (auth.isAdmin) load();
    });

    function describe(e: unknown): string {
        if (e instanceof ApiError) return `${e.message} (${e.code})`;
        return e instanceof Error ? e.message : String(e);
    }

    async function handleDeleteConfirm() {
        if (!deleting) return;
        const target = deleting;
        deleteBusy = true;
        actionError = '';
        try {
            await deleteUser(target.id);
            users = users.filter((u) => u.id !== target.id);
            deleting = null;
        } catch (e) {
            actionError = `Could not delete “${target.email}”: ${describe(e)}`;
        } finally {
            deleteBusy = false;
        }
    }

    function handleSaved() {
        createOpen = false;
        editing = null;
        load();
    }
</script>

<svelte:head>
    <title>GIS Sentinel — Users</title>
</svelte:head>

{#if adminChecked}
    <div class="flex flex-col gap-4">
        <div class="flex flex-wrap items-center justify-between gap-3">
            <h1 class="text-2xl font-bold">Users</h1>
            <button
                type="button"
                onclick={() => (createOpen = true)}
                class="rounded-md bg-sky-600 px-4 py-2 text-sm font-semibold text-white hover:bg-sky-500"
            >
                + New user
            </button>
        </div>

        {#if actionError}
            <div
                class="rounded-md bg-rose-500/10 px-3 py-2 text-sm text-rose-600 dark:text-rose-400"
            >
                {actionError}
            </div>
        {/if}

        {#if loading}
            <div class="space-y-2">
                {#each [0, 1, 2] as i (i)}
                    <div class="h-12 animate-pulse rounded-lg bg-surface-2"></div>
                {/each}
            </div>
        {:else if loadError}
            <div class="rounded-lg border border-rose-500/40 bg-rose-500/10 p-6 text-center">
                <div class="font-medium text-rose-600 dark:text-rose-400">Failed to load users</div>
                <div class="mt-1 text-sm text-muted">{loadError}</div>
                <button
                    type="button"
                    onclick={load}
                    class="mt-3 rounded-md border border-line px-3 py-1.5 text-sm hover:bg-surface-2"
                >
                    Retry
                </button>
            </div>
        {:else if users.length === 0}
            <div
                class="flex flex-col items-center gap-2 rounded-lg border border-dashed border-line bg-surface-2 p-12 text-center"
            >
                <div class="font-medium">No users yet</div>
                <div class="text-sm text-muted">Add the first account to get started.</div>
            </div>
        {:else}
            <UsersTable
                {users}
                {busyId}
                currentUserId={auth.currentUser?.id ?? null}
                onedit={(user) => (editing = user)}
                ondelete={(user) => (deleting = user)}
            />
            <div class="text-xs text-muted">
                {users.length} user{users.length === 1 ? '' : 's'} · role changes and password
                resets sign the affected user out everywhere
            </div>
        {/if}
    </div>

    {#if createOpen}
        <UserCreateModal
            onClose={() => (createOpen = false)}
            onSaved={(user) => {
                createOpen = false;
                users = [...users, user];
            }}
        />
    {/if}

    {#if editing}
        <UserEditModal
            user={editing}
            currentUserId={auth.currentUser?.id ?? null}
            onClose={() => (editing = null)}
            onSaved={handleSaved}
        />
    {/if}

    {#if deleting}
        <UserDeleteConfirm
            user={deleting}
            currentUserId={auth.currentUser?.id ?? null}
            busy={deleteBusy}
            onConfirm={handleDeleteConfirm}
            onCancel={() => (deleting = null)}
        />
    {/if}
{/if}
