<script lang="ts">
    import { onMount } from 'svelte';
    import { getAlertPointHistory, listAlertPoints, ApiError } from '$lib/api';
    import type { AlertPoint, ProbeHistory } from '$lib/types';
    import IncidentTable from '$lib/components/IncidentTable.svelte';
    import LatencyChart from '$lib/components/LatencyChart.svelte';
    import UptimeStats from '$lib/components/UptimeStats.svelte';

    const WINDOWS = [24, 168, 720] as const;
    const WINDOW_LABELS: Record<number, string> = { 24: '24 h', 168: '7 d', 720: '30 d' };

    let points = $state<AlertPoint[]>([]);
    let selectedId = $state<string | null>(null);
    let loadingPoints = $state(true);

    let histories = $state<Record<number, ProbeHistory | null>>({});
    let loadingHistory = $state(false);
    let historyError = $state('');

    let selectedWindow = $state<number>(24);

    const selected = $derived(points.find((p) => p.id === selectedId) ?? null);

    onMount(async () => {
        try {
            const res = await listAlertPoints({ per_page: 200 });
            points = res.data;
            if (res.data.length > 0) selectedId = res.data[0].id;
        } catch (e) {
            historyError = e instanceof Error ? e.message : String(e);
        } finally {
            loadingPoints = false;
        }
    });

    // Load all three windows whenever the selected point changes.
    $effect(() => {
        const id = selectedId;
        if (!id) return;
        loadingHistory = true;
        historyError = '';
        Promise.all(WINDOWS.map((h) => getAlertPointHistory(id, h)))
            .then((results) => {
                const next: Record<number, ProbeHistory | null> = {};
                WINDOWS.forEach((h, i) => (next[h] = results[i]));
                histories = next;
            })
            .catch((e) => {
                historyError =
                    e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
            })
            .finally(() => (loadingHistory = false));
    });

    const current = $derived(selectedId ? (histories[selectedWindow] ?? null) : null);
</script>

<svelte:head>
    <title>GIS Sentinel — History</title>
</svelte:head>

<div class="flex flex-col gap-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-2xl font-bold">History</h1>
        <div class="flex items-center gap-2">
            <select
                bind:value={selectedId}
                aria-label="Select monitoring point"
                class="rounded-md border border-line bg-surface px-2 py-1.5 text-sm"
            >
                {#each points as p (p.id)}
                    <option value={p.id}>{p.name}</option>
                {/each}
            </select>
            <select
                bind:value={selectedWindow}
                aria-label="Time window"
                class="rounded-md border border-line bg-surface px-2 py-1.5 text-sm"
            >
                {#each WINDOWS as h (h)}
                    <option value={h}>{WINDOW_LABELS[h]}</option>
                {/each}
            </select>
        </div>
    </div>

    {#if loadingPoints}
        <div class="space-y-2">
            {#each [0, 1, 2] as i (i)}
                <div class="h-12 animate-pulse rounded-lg bg-surface-2"></div>
            {/each}
        </div>
    {:else if points.length === 0}
        <div
            class="rounded-lg border border-dashed border-line bg-surface-2 p-12 text-center text-sm text-muted"
        >
            Nothing monitored yet — add a service on the Services page first.
        </div>
    {:else}
        {#if historyError}
            <div
                class="rounded-md bg-rose-500/10 px-3 py-2 text-sm text-rose-600 dark:text-rose-400"
            >
                {historyError}
            </div>
        {/if}

        {#if selected}
            <div class="flex flex-wrap items-center gap-2">
                <span class="font-semibold">{selected.name}</span>
                <span class="font-mono text-xs text-muted">{selected.url}</span>
            </div>
        {/if}

        <UptimeStats pointId={selectedId ?? ''} />

        {#if loadingHistory}
            <div class="h-64 animate-pulse rounded-lg bg-surface-2"></div>
        {:else if current}
            <LatencyChart
                probes={current.probes}
                expected={selected?.expected_response_time_ms ?? null}
                hours={current.hours}
            />
            <IncidentTable pointId={selectedId ?? ''} hours={selectedWindow} />
            {#if current.probes.length >= 5000}
                <div class="text-xs text-muted">
                    Showing the most recent 5000 probes (row cap) for this window.
                </div>
            {/if}
        {/if}
    {/if}
</div>
