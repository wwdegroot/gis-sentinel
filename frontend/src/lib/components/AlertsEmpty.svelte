<script lang="ts">
    /** Empty state: skeleton while connecting, otherwise the all-clear. */
    let { loading = false }: { loading?: boolean } = $props();
</script>

<div
    class="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-line bg-surface-2 p-12 text-center"
>
    {#if loading}
        <div class="text-3xl">🛰️</div>
        <div class="font-medium text-text">Connecting to sentinel…</div>
        <div class="text-sm text-muted">Waiting for the first snapshot from the backend.</div>
        <div class="mt-2 flex gap-2">
            <div class="h-3 w-24 animate-pulse rounded bg-line"></div>
            <div class="h-3 w-16 animate-pulse rounded bg-line"></div>
        </div>
    {:else}
        <div class="text-3xl">✅</div>
        <div class="font-medium text-text">No active incidents</div>
        <div class="text-sm text-muted">All monitored services are currently healthy.</div>
    {/if}
</div>
