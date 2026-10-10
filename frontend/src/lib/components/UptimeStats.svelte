<script lang="ts">
    import { getAlertPointHistory } from '$lib/api';

    let { pointId }: { pointId: string } = $props();

    const WINDOWS = [
        { hours: 24, label: '24 h' },
        { hours: 168, label: '7 d' },
        { hours: 720, label: '30 d' }
    ];

    let uptime = $state<Record<number, number | null>>({});
    let loading = $state(true);
    let error = $state('');

    async function load(id: string) {
        loading = true;
        error = '';
        try {
            const results = await Promise.all(
                WINDOWS.map((w) => getAlertPointHistory(id, w.hours))
            );
            const next: Record<number, number | null> = {};
            WINDOWS.forEach((w, i) => (next[w.hours] = results[i].uptime_pct));
            uptime = next;
        } catch (e) {
            error = e instanceof Error ? e.message : String(e);
        } finally {
            loading = false;
        }
    }

    // Re-fetch whenever the selected point changes (also covers the first load).
    $effect(() => {
        load(pointId);
    });
</script>

<div class="grid grid-cols-3 gap-3">
    {#each WINDOWS as w (w.hours)}
        <div class="rounded-lg border border-line bg-surface p-4 text-center">
            <div class="text-xs font-medium uppercase tracking-wide text-muted">
                Uptime {w.label}
            </div>
            <div
                class="mt-1 text-2xl font-bold {loading ? 'animate-pulse text-muted' : 'text-text'}"
            >
                {#if loading}
                    …
                {:else if error || uptime[w.hours] === undefined}
                    —
                {:else if uptime[w.hours] === null}
                    n/a
                {:else}
                    {uptime[w.hours]!.toFixed(2)}%
                {/if}
            </div>
        </div>
    {/each}
    {#if error}
        <div class="col-span-3 text-xs text-rose-500">{error}</div>
    {/if}
</div>
