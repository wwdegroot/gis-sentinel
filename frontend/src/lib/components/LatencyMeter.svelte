<script lang="ts">
    /**
     * Latency bar for one probe: actual vs expected SLA. The track spans
     * 0–2× the expected latency; the fill color degrades with the ratio.
     */
    let { response_time_ms, expected }: { response_time_ms: number | null; expected: number } =
        $props();

    const ratio = $derived(response_time_ms === null ? null : response_time_ms / expected);
    // 100% fill == 2× expected latency.
    const pct = $derived(ratio === null ? 0 : Math.min(100, (ratio / 2) * 100));
    const color = $derived(
        ratio === null
            ? 'bg-slate-400'
            : ratio <= 1
              ? 'bg-emerald-500'
              : ratio <= 1.5
                ? 'bg-amber-500'
                : 'bg-rose-500'
    );
</script>

<div class="flex flex-col gap-1">
    <div class="flex justify-between text-xs text-muted">
        <span>latency</span>
        <span class="font-mono">
            {response_time_ms ?? '—'} / {expected} ms
            {#if ratio !== null && ratio > 1}
                <span class="text-rose-500 dark:text-rose-400"
                    >({Math.round(ratio * 100)}% of SLA)</span
                >
            {/if}
        </span>
    </div>
    <div class="h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
        <div
            class="h-full rounded-full transition-all duration-500 {color}"
            style="width: {pct}%"
        ></div>
    </div>
    <div class="flex justify-between text-[10px] text-muted">
        <span>0</span>
        <span>SLA {expected}</span>
        <span>2×</span>
    </div>
</div>
