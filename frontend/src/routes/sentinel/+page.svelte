<script lang="ts">
	import { SentinelSocket } from "$lib/sentinelSocket.svelte";
	import type { SentinelAlert } from "$lib/types";
	import { onDestroy, onMount } from "svelte";

    let socket: SentinelSocket = new SentinelSocket(`ws://${location.hostname}:3000/ws/sentinel`)

	onMount(() => {
        socket.connect() 
        //socket.test()
	});

	onDestroy(() => {
		if (socket) {
			socket.close();
		}

	});
</script>

{#snippet alertcard(alert: SentinelAlert, classNames: string)}
<div class={classNames}>
    {#each Object.entries(alert) as [key, value]}
        {key} = {value}
        <hr>
    {/each}
</div>
{/snippet}

<div class="container mx-auto text-center font-bold text-3xl">GIS Sentinel</div>
<div class="container mx-auto">
    <div>Connection Status: {#if socket.connection}🟢{:else}🔴{/if}</div>
    <div class="flex gap-4">
        {#each socket.alerts as alert}
            {#if alert.performance > alert.expected && alert.up}
                {@render alertcard(alert, "bg-orange-500 hover:bg-orange-700 p-4 rounded-md")}
            {:else} 
                {@render alertcard(alert, "bg-rose-500 hover:bg-rose-700 p-4 rounded-md")}
            {/if}
       {/each}
    </div>

</div>
