<script lang="ts">
    import type { ProbeObservation } from '$lib/types';

    let {
        targetName,
        running = false,
        result = null,
        error = ''
    }: {
        targetName: string;
        running: boolean;
        result: ProbeObservation | null;
        error: string;
    } = $props();
</script>

<div class="rounded-lg border border-line bg-surface-2 p-4">
    <div class="mb-2 flex items-center justify-between">
        <span class="text-sm font-medium">
            On-demand probe — <span class="text-muted">{targetName}</span>
        </span>
        {#if running}
            <span class="text-xs text-muted animate-pulse">probing…</span>
        {/if}
    </div>

    {#if error}
        <div class="rounded-md bg-rose-500/10 px-3 py-2 text-sm text-rose-600 dark:text-rose-400">
            {error}
        </div>
    {:else if running}
        <div class="h-3 w-full max-w-xs animate-pulse rounded bg-line"></div>
    {:else if result}
        <div class="flex flex-wrap items-center gap-3 text-sm">
            <span
                class="rounded-full px-2 py-0.5 text-xs font-semibold uppercase {result.is_up
                    ? 'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400'
                    : 'bg-rose-500/15 text-rose-600 dark:text-rose-400'}"
            >
                {result.is_up ? 'Up' : 'Down'}
            </span>
            {#if result.status_code !== null}
                <span class="font-mono text-xs text-muted">HTTP {result.status_code}</span>
            {/if}
            {#if result.response_time_ms !== null}
                <span class="font-mono text-xs text-muted">{result.response_time_ms} ms</span>
            {/if}
        </div>
        {#if result.error_message}
            <div class="mt-2 text-sm text-rose-600 dark:text-rose-400">{result.error_message}</div>
        {/if}
        {#if result.raw_response_snippet}
            <details class="mt-2">
                <summary class="cursor-pointer text-xs text-muted">response snippet</summary>
                <pre
                    class="mt-1 max-h-40 overflow-auto rounded bg-surface p-2 font-mono text-[11px] text-muted">{result.raw_response_snippet}</pre>
            </details>
        {/if}
        <div class="mt-2 text-[11px] text-muted">
            Diagnostic only — not persisted, raises no alerts.
        </div>
    {/if}
</div>
