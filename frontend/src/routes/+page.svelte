<script lang="ts">
    import { sentinel } from '$lib/sentinelSocket.svelte';
    import type { ServiceStatus, ServiceType } from '$lib/types';
    import AlertCard from '$lib/components/AlertCard.svelte';
    import AlertsEmpty from '$lib/components/AlertsEmpty.svelte';
    import SummaryStats from '$lib/components/SummaryStats.svelte';

    // Live filters (client-side over the socket state, per plan §3.2).
    let statusFilter: 'all' | ServiceStatus = $state('all');
    let typeFilter: 'all' | ServiceType = $state('all');
    let search = $state('');

    const serviceTypes: ServiceType[] = ['WMS', 'WFS', 'WMTS', 'OAF', 'ArcGIS_REST', 'HTTP'];

    const severity: Record<ServiceStatus, number> = { down: 0, degraded: 1, healthy: 2 };

    const filtered = $derived(
        [...sentinel.alerts]
            .filter((a) => statusFilter === 'all' || a.status === statusFilter)
            .filter((a) => typeFilter === 'all' || a.service_type === typeFilter)
            .filter((a) => {
                const q = search.trim().toLowerCase();
                return (
                    q === '' || a.name.toLowerCase().includes(q) || a.url.toLowerCase().includes(q)
                );
            })
            .sort(
                (a, b) =>
                    severity[a.status] - severity[b.status] ||
                    Date.parse(b.triggered_at) - Date.parse(a.triggered_at)
            )
    );

    const hasFilters = $derived(
        statusFilter !== 'all' || typeFilter !== 'all' || search.trim() !== ''
    );
</script>

<svelte:head>
    <title>GIS Sentinel — Dashboard</title>
</svelte:head>

<div class="flex flex-col gap-6">
    <div class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-2xl font-bold">Live monitoring</h1>
        <div class="text-xs text-muted">
            {sentinel.alerts.length}
            open alert{sentinel.alerts.length === 1 ? '' : 's'} · updates arrive in real time
        </div>
    </div>

    <SummaryStats alerts={sentinel.alerts} />

    <div class="flex flex-wrap items-center gap-2">
        <select
            bind:value={statusFilter}
            aria-label="Filter by status"
            class="rounded-md border border-line bg-surface px-2 py-1.5 text-sm"
        >
            <option value="all">All statuses</option>
            <option value="down">Down</option>
            <option value="degraded">Degraded</option>
        </select>

        <select
            bind:value={typeFilter}
            aria-label="Filter by service type"
            class="rounded-md border border-line bg-surface px-2 py-1.5 text-sm"
        >
            <option value="all">All service types</option>
            {#each serviceTypes as type (type)}
                <option value={type}>{type.replace(/_/g, ' ')}</option>
            {/each}
        </select>

        <input
            type="search"
            placeholder="Search name or URL…"
            bind:value={search}
            aria-label="Search alerts"
            class="min-w-48 flex-1 rounded-md border border-line bg-surface px-3 py-1.5 text-sm placeholder:text-muted"
        />

        {#if hasFilters}
            <button
                type="button"
                onclick={() => {
                    statusFilter = 'all';
                    typeFilter = 'all';
                    search = '';
                }}
                class="rounded-md border border-line px-3 py-1.5 text-sm text-muted hover:text-text"
            >
                Clear filters
            </button>
        {/if}
    </div>

    {#if sentinel.alerts.length === 0}
        <AlertsEmpty loading={sentinel.connection !== 'connected'} />
    {:else if filtered.length === 0}
        <div
            class="rounded-lg border border-dashed border-line bg-surface-2 p-10 text-center text-sm text-muted"
        >
            No alerts match the current filters.
        </div>
    {:else}
        <div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
            {#each filtered as alert (alert.alert_id)}
                <AlertCard {alert} />
            {/each}
        </div>
    {/if}
</div>
