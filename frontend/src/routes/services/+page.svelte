<script lang="ts">
    import { onMount } from 'svelte';
    import {
        deleteAlertPoint,
        listAlertPoints,
        testAlertPoint,
        updateAlertPoint,
        ApiError
    } from '$lib/api';
    import type { AlertPoint, ProbeObservation } from '$lib/types';
    import AlertPointModal from '$lib/components/AlertPointModal.svelte';
    import DeleteConfirm from '$lib/components/DeleteConfirm.svelte';
    import ServiceTable from '$lib/components/ServiceTable.svelte';
    import TestResultPanel from '$lib/components/TestResultPanel.svelte';

    let points = $state<AlertPoint[]>([]);
    let loading = $state(true);
    let loadError = $state('');

    // modal / dialog state
    let modalOpen = $state(false);
    let editing = $state<AlertPoint | null>(null);
    let deleting = $state<AlertPoint | null>(null);
    let deleteBusy = $state(false);

    // on-demand probe
    let testingId = $state<string | null>(null);
    let testTarget = $state<{
        name: string;
        observation: ProbeObservation | null;
        error: string;
    } | null>(null);

    // in-flight mutation (toggle/delete)
    let busyId = $state<string | null>(null);
    let actionError = $state('');

    async function load() {
        loading = true;
        loadError = '';
        try {
            const res = await listAlertPoints({ per_page: 200 });
            points = res.data;
        } catch (e) {
            loadError = describe(e);
        } finally {
            loading = false;
        }
    }

    onMount(load);

    function describe(e: unknown): string {
        if (e instanceof ApiError) return `${e.message} (${e.code})`;
        return e instanceof Error ? e.message : String(e);
    }

    /** Optimistic enable/disable with rollback on failure. */
    async function handleToggle(point: AlertPoint, enabled: boolean) {
        const previous = point.enabled;
        point.enabled = enabled;
        busyId = point.id;
        actionError = '';
        try {
            await updateAlertPoint(point.id, { enabled });
        } catch (e) {
            point.enabled = previous;
            actionError = `Could not update “${point.name}”: ${describe(e)}`;
        } finally {
            busyId = null;
        }
    }

    async function handleTest(point: AlertPoint) {
        testingId = point.id;
        testTarget = { name: point.name, observation: null, error: '' };
        try {
            const observation = await testAlertPoint(point.id);
            testTarget = { name: point.name, observation, error: '' };
        } catch (e) {
            testTarget = { name: point.name, observation: null, error: describe(e) };
        } finally {
            testingId = null;
        }
    }

    async function handleDeleteConfirm() {
        if (!deleting) return;
        const target = deleting;
        deleteBusy = true;
        actionError = '';
        try {
            await deleteAlertPoint(target.id);
            points = points.filter((p) => p.id !== target.id);
            deleting = null;
        } catch (e) {
            actionError = `Could not delete “${target.name}”: ${describe(e)}`;
        } finally {
            deleteBusy = false;
        }
    }

    function handleSaved() {
        modalOpen = false;
        editing = null;
        load();
    }
</script>

<svelte:head>
    <title>GIS Sentinel — Services</title>
</svelte:head>

<div class="flex flex-col gap-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-2xl font-bold">Services</h1>
        <button
            type="button"
            onclick={() => {
                editing = null;
                modalOpen = true;
            }}
            class="rounded-md bg-sky-600 px-4 py-2 text-sm font-semibold text-white hover:bg-sky-500"
        >
            + New monitoring point
        </button>
    </div>

    {#if actionError}
        <div class="rounded-md bg-rose-500/10 px-3 py-2 text-sm text-rose-600 dark:text-rose-400">
            {actionError}
        </div>
    {/if}

    {#if testTarget}
        <TestResultPanel
            targetName={testTarget.name}
            running={testingId !== null}
            result={testTarget.observation}
            error={testTarget.error}
        />
    {/if}

    {#if loading}
        <div class="space-y-2">
            {#each [0, 1, 2] as i (i)}
                <div class="h-12 animate-pulse rounded-lg bg-surface-2"></div>
            {/each}
        </div>
    {:else if loadError}
        <div class="rounded-lg border border-rose-500/40 bg-rose-500/10 p-6 text-center">
            <div class="font-medium text-rose-600 dark:text-rose-400">Failed to load services</div>
            <div class="mt-1 text-sm text-muted">{loadError}</div>
            <button
                type="button"
                onclick={load}
                class="mt-3 rounded-md border border-line px-3 py-1.5 text-sm hover:bg-surface-2"
            >
                Retry
            </button>
        </div>
    {:else if points.length === 0}
        <div
            class="flex flex-col items-center gap-2 rounded-lg border border-dashed border-line bg-surface-2 p-12 text-center"
        >
            <div class="text-3xl">🛰️</div>
            <div class="font-medium">No monitoring points yet</div>
            <div class="text-sm text-muted">Add your first GIS endpoint to start monitoring.</div>
            <button
                type="button"
                onclick={() => {
                    editing = null;
                    modalOpen = true;
                }}
                class="mt-2 rounded-md bg-sky-600 px-4 py-2 text-sm font-semibold text-white hover:bg-sky-500"
            >
                + New monitoring point
            </button>
        </div>
    {:else}
        <ServiceTable
            {points}
            {busyId}
            {testingId}
            ontoggle={handleToggle}
            ontest={handleTest}
            onedit={(point) => {
                editing = point;
                modalOpen = true;
            }}
            ondelete={(point) => (deleting = point)}
        />
        <div class="text-xs text-muted">
            {points.length} point{points.length === 1 ? '' : 's'} configured
        </div>
    {/if}
</div>

{#if modalOpen}
    <AlertPointModal
        point={editing}
        onClose={() => {
            modalOpen = false;
            editing = null;
        }}
        onSaved={handleSaved}
    />
{/if}

{#if deleting}
    <DeleteConfirm
        point={deleting}
        busy={deleteBusy}
        onConfirm={handleDeleteConfirm}
        onCancel={() => (deleting = null)}
    />
{/if}
