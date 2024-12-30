<script lang="ts">
	import { onDestroy, onMount } from 'svelte';

	let socket: WebSocket;

    let messages: string[] = $state([])

    let timeoutID: number = 0;
    let start: boolean = $state(false);

    function startMessaging() {
        start = true;
        socket = new WebSocket(`ws://${location.hostname}/ws`);
        socket.addEventListener('open', function (event) {
            socket.send('Hello Server!');
        });

        socket.addEventListener('message', function (event) {
            messages.push(`Message from server: ${event.data}`)
        });

        socket.addEventListener('close', function(event) {
            messages.push(`Closing websocket: ${event.reason}`)
            start = false;
        })
        timeoutID = setTimeout(() => {
            const obj = { hello: "world" };
            const blob = new Blob([JSON.stringify(obj, null, 2)], {
                type: "application/json",
            });
            socket.send(blob);
        }, 1000);
    }

    function stopMessaging() {
        clearTimeout(timeoutID)
        start = false;
        socket.close(3000, "Crash and Burn!")
    }


    function clearMessages() {
        messages = []
    }

	onMount(() => {
		


	});

	onDestroy(() => {
		if (socket) {
			socket.close();
		}

	});

</script>

<div class="container mx-auto">
    <div class="mt-4 flex flex-row gap-2 max-h-2/4">
        <div class="flex flex-col gap-2 min-w-20">
            <div>Connection: {#if start}🟢{:else}🔴{/if}</div>
            <div>
                
                <button class="bg-lime-300 rounded-md p-2 text-lime-700 font-bold min-w-16" onclick={() => startMessaging()}>Start</button>
            </div>
            <div>
                <button class="bg-red-300 rounded-md p-2 text-red-700 font-bold min-w-16" onclick={() => stopMessaging()} >Stop</button>
            </div>
            <div>
                <button class="bg-violet-300 rounded-md p-2 text-violet-700 font-bold min-w-16" onclick={() => clearMessages()} >Clear</button>
            </div>            
        </div>
        <div class="bg-slate-700 rounded-md p-2 min-w-96 max-h-96 overflow-y-auto">
            {#each messages.slice().reverse() as message}
                <hr>
                <div class="text-white">{message}</div>
            {/each}
        </div>
    </div>
</div>