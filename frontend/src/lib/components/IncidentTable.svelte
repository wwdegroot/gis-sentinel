<script lang="ts">
    import { getAlertPointIncidents, exportHistoryUrl, ApiError } from '$lib/api';
    import { formatDateTime } from '$lib/format';
    import type { ProbeIncident } from '$lib/types';

    /**
     * Failed probes for one point/window, fetched from the dedicated
     * `GET /{id}/incidents` endpoint (the history endpoint returns a slim
     * graph-only payload without error detail). Export buttons download
     * straight from the backend (`/{id}/history/export`).
     */
    let { pointId, hours }: { pointId: string; hours: number } = $props();

    let incidents = $state<ProbeIncident[]>([]);
    let loading = $state(true);
    let error = $state('');

    async function load(id: string, hours: number) {
        loading = true;
        error = '';
        try {
            incidents = await getAlertPointIncidents(id, hours);
        } catch (e) {
            error = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
            incidents = [];
        } finally {
            loading = false;
        }
    }

    // Re-fetch whenever the point or the window changes.
    $effect(() => {
        load(pointId, hours);
    });

    function download(format: 'csv' | 'json') {
        const a = document.createElement('a');
        a.href = exportHistoryUrl(pointId, hours, format);
        a.download = '';
        document.body.appendChild(a);
        a.click();
        a.remove();
    }
</script>

<div class="rounded-lg border border-line bg-surface">
    <div class="flex items-center justify-between border-b border-line px-4 py-3">
        <span class="text-sm font-medium">
            Failed probes ({incidents.length}
            in window)
        </span>
        <div class="flex gap-2">
            <button
                type="button"
                onclick={() => download('csv')}
                disabled={loading }
                class="rounded-md border border-line px-2 py-1 text-xs hover:bg-surface-2 disabled:opacity-40"
            >
                Export CSV
            </button>
            <button
                type="button"
                onclick={() => download('json')}
                disabled={loading }
                class="rounded-md border border-line px-2 py-1 text-xs hover:bg-surface-2 disabled:opacity-40"
            >
                Export JSON
            </button>
        </div>
    </div>

    {#if loading}
        <div class="space-y-2 p-4">
            {#each [0, 1, 2] as i (i)}
                <div class="h-8 animate-pulse rounded bg-surface-2"></div>
            {/each}
        </div>
    {:else if error}
        <div class="px-4 py-3 text-sm text-rose-600 dark:text-rose-400">{error}</div>
    {:else if incidents.length === 0}
        <div class="p-6 text-center text-sm text-muted">No failed probes in this window. 🎉</div>
    {:else}
        <div class="max-h-80 overflow-y-auto">
            <table class="w-full text-left text-sm">
                <thead class="sticky top-0 bg-surface-2 text-xs uppercase tracking-wide text-muted">
                    <tr>
                        <th class="px-4 py-2">Timestamp</th>
                        <th class="px-4 py-2">HTTP</th>
                        <th class="px-4 py-2">Latency</th>
                        <th class="px-4 py-2">Error</th>
                    </tr>
                </thead>
                <tbody>
                    {#each incidents as p (p.timestamp)}
                        <tr class="border-t border-line">
                            <td class="px-4 py-2 text-xs text-muted"
                                >{formatDateTime(p.timestamp)}</td
                            >
                            <td class="px-4 py-2 font-mono text-xs">
                                {p.status_code ?? '—'}
                            </td>
                            <td class="px-4 py-2 font-mono text-xs">
                                {p.response_time_ms ?? '—'}
                            </td>
                            <td
                                class="max-w-96 truncate px-4 py-2 text-xs text-rose-600 dark:text-rose-400"
                                title={p.error_message ?? ''}
                            >
                                {p.error_message ?? '—'}
                            </td>
                        </tr>
                    {/each}
                </tbody>
            </table>
        </div>
    {/if}
</div>
