<script lang="ts">
    import { sentinel } from '$lib/sentinelSocket.svelte';

    const dot = $derived(
        sentinel.connection === 'connected'
            ? 'bg-emerald-500'
            : sentinel.reconnecting
              ? 'bg-amber-500 animate-pulse'
              : 'bg-rose-500'
    );
    const label = $derived(
        sentinel.connection === 'connected'
            ? 'Connected'
            : sentinel.reconnecting
              ? `Reconnecting (${sentinel.retryCount})`
              : sentinel.connection === 'connecting'
                ? 'Connecting…'
                : 'Disconnected'
    );
</script>

<span
    class="inline-flex items-center gap-1.5 rounded-full bg-surface-2 px-2.5 py-1 text-xs font-medium text-muted ring-1 ring-line"
    title="Sentinel WebSocket connection"
>
    <span class="h-2 w-2 rounded-full {dot}"></span>
    {label}
</span>
