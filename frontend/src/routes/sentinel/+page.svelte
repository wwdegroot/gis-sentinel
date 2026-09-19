<script lang="ts">
    import { SentinelSocket } from '$lib/sentinelSocket.svelte';
    import { sentinel } from '$lib/sentinelSocket.svelte';
    import type { AlertEvent } from '$lib/types';
    import { onDestroy, onMount } from 'svelte';

    // Minimal interim view (full redesign lands in task 3.3). Uses the shared
    // singleton so the socket stays alive across navigations.
    const socket: SentinelSocket = sentinel;

    onMount(() => {
        socket.connect();
    });

    let ticker: ReturnType<typeof setInterval>;
    function startTicker() {
        ticker = setInterval(() => {}, 1000);
    }
    onMount(startTicker);
    onDestroy(() => clearInterval(ticker));
</script>

{#snippet alertcard(alert: AlertEvent, classNames: string)}
    <div class={classNames}>
        <div class="font-bold">{alert.name}</div>
        <div class="text-sm opacity-80">{alert.url}</div>
        <div>
            {alert.status} · alert_type={alert.alert_type} · latency={alert.response_time_ms ??
                '—'}ms (SLA {alert.expected_response_time_ms}ms)
        </div>
        <div class="text-sm">{alert.reason}</div>
        <div class="text-xs opacity-60">since {new Date(alert.triggered_at).toLocaleString()}</div>
    </div>
{/snippet}

<div class="container mx-auto text-center font-bold text-3xl">GIS Sentinel</div>
<div class="container mx-auto">
    <div>
        Connection Status: {#if socket.reconnecting}🟡{:else if socket.connection === 'connected'}🟢{:else}🔴{/if}
        {#if socket.reconnecting}(retry #{socket.retryCount}){/if}
    </div>
    <div class="flex gap-4">
        {#each socket.alerts as alert (alert.alert_id)}
            {#if alert.status === 'down'}
                {@render alertcard(
                    alert,
                    'bg-rose-500 hover:bg-rose-700 p-4 rounded-md text-white'
                )}
            {:else}
                {@render alertcard(
                    alert,
                    'bg-orange-500 hover:bg-orange-700 p-4 rounded-md text-white'
                )}
            {/if}
        {/each}
    </div>
</div>
