<script lang="ts">
    import { onMount } from 'svelte';
    import { listAlertPoints } from '$lib/api';
    import type { AlertEvent } from '$lib/types';

    let { alerts }: { alerts: AlertEvent[] } = $props();

    /** Configured targets, from the REST API (independent of open alerts). */
    let total = $state<number | null>(null);

    onMount(() => {
        load();
        const timer = setInterval(load, 30_000);
        return () => clearInterval(timer);
    });

    async function load() {
        try {
            const res = await listAlertPoints({ per_page: 1 });
            total = res.pagination.total;
        } catch (e) {
            console.error('failed to load alert point count', e);
        }
    }

    const down = $derived(alerts.filter((a) => a.status === 'down').length);
    const degraded = $derived(alerts.filter((a) => a.status === 'degraded').length);
    const incidents = $derived(alerts.length);
    const healthy = $derived(total === null ? null : Math.max(0, total - incidents));
    const avgLatency = $derived.by(() => {
        const values = alerts.map((a) => a.response_time_ms).filter((v): v is number => v !== null);
        if (values.length === 0) return null;
        return Math.round(values.reduce((sum, v) => sum + v, 0) / values.length);
    });
</script>

<div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
    <div class="rounded-lg border border-line bg-surface p-4">
        <div class="text-xs font-medium uppercase tracking-wide text-muted">Total monitored</div>
        <div class="mt-1 text-2xl font-bold text-text">{total ?? '—'}</div>
    </div>
    <div class="rounded-lg border border-line bg-surface p-4">
        <div class="text-xs font-medium uppercase tracking-wide text-muted">Healthy</div>
        <div class="mt-1 text-2xl font-bold text-emerald-500">{healthy ?? '—'}</div>
    </div>
    <div class="rounded-lg border border-line bg-surface p-4">
        <div class="text-xs font-medium uppercase tracking-wide text-muted">Active incidents</div>
        <div class="mt-1 text-2xl font-bold {incidents > 0 ? 'text-rose-500' : 'text-text'}">
            {incidents}
            {#if down > 0 || degraded > 0}
                <span class="text-sm font-medium text-muted">
                    ({down} down, {degraded} degraded)
                </span>
            {/if}
        </div>
    </div>
    <div class="rounded-lg border border-line bg-surface p-4">
        <div class="text-xs font-medium uppercase tracking-wide text-muted">Avg latency</div>
        <div class="mt-1 text-2xl font-bold text-text">
            {avgLatency === null ? '—' : `${avgLatency} ms`}
        </div>
    </div>
</div>
