<script lang="ts">
    import { formatDateTime } from '$lib/format';
    import type { ProbeHistoryPoint } from '$lib/types';

    /**
     * Hand-rolled SVG latency chart (plan Q2-a: zero dependencies).
     *
     * - x axis spans the requested window (gaps without probes show as gaps)
     * - y axis spans 0..max(observed latency, 1.25 × SLA)
     * - dashed line marks the SLA; failed probes are red marks at the bottom
     */
    let {
        probes,
        expected = null,
        hours,
        width = 800,
        height = 280
    }: {
        probes: ProbeHistoryPoint[];
        /** SLA in ms (dashed reference line). */
        expected?: number | null;
        /** Window length in hours (x-axis span). */
        hours: number;
        width?: number;
        height?: number;
    } = $props();

    const pad = { top: 14, right: 16, bottom: 26, left: 56 };

    const windowStartMs = $derived(Date.now() - hours * 3_600_000);
    const windowEndMs = $derived(Date.now());

    const okPoints = $derived(
        probes.filter((p) => p.is_up && p.response_time_ms !== null && p.response_time_ms > 0)
    );
    const failedPoints = $derived(probes.filter((p) => !p.is_up));

    const yMax = $derived.by(() => {
        const observed = okPoints.reduce((m, p) => Math.max(m, p.response_time_ms ?? 0), 0);
        const sla = expected ?? 0;
        return Math.max(observed * 1.15, sla * 1.25, 1);
    });

    const x = $derived.by(() => {
        const span = windowEndMs - windowStartMs;
        return (t: string) =>
            pad.left + ((Date.parse(t) - windowStartMs) / span) * (width - pad.left - pad.right);
    });
    const y = $derived.by(() => {
        const innerH = height - pad.top - pad.bottom;
        return (ms: number) => pad.top + innerH - (ms / yMax) * innerH;
    });

    const linePath = $derived.by(() => {
        if (okPoints.length === 0) return '';
        return okPoints
            .map(
                (p, i) =>
                    `${i === 0 ? 'M' : 'L'}${x(p.timestamp).toFixed(1)},${y(
                        p.response_time_ms as number
                    ).toFixed(1)}`
            )
            .join(' ');
    });

    const gridLines = $derived([0, 0.25, 0.5, 0.75, 1].map((f) => ({ f, ms: yMax * f })));
    const slaY = $derived(expected !== null && expected <= yMax ? y(expected) : null);
</script>

<div class="overflow-x-auto rounded-lg border border-line bg-surface p-2">
    {#if probes.length === 0}
        <div class="p-10 text-center text-sm text-muted">
            No probe data in the last {hours} h.
        </div>
    {:else}
        <svg
            viewBox="0 0 {width} {height}"
            class="w-full"
            role="img"
            aria-label="Latency over the last {hours} hours"
        >
            <!-- grid -->
            {#each gridLines as g (g.f)}
                <line
                    x1={pad.left}
                    x2={width - pad.right}
                    y1={y(g.ms)}
                    y2={y(g.ms)}
                    class="stroke-line"
                    stroke-width="1"
                    stroke-dasharray="2 4"
                />
                <text
                    x={pad.left - 6}
                    y={y(g.ms) + 3}
                    text-anchor="end"
                    class="fill-muted text-[9px]"
                >
                    {Math.round(g.ms)}ms
                </text>
            {/each}

            <!-- SLA line -->
            {#if slaY !== null}
                <line
                    x1={pad.left}
                    x2={width - pad.right}
                    y1={slaY}
                    y2={slaY}
                    class="stroke-amber-500"
                    stroke-width="1.5"
                    stroke-dasharray="6 4"
                />
                <text
                    x={width - pad.right}
                    y={slaY - 4}
                    text-anchor="end"
                    class="fill-amber-500 text-[10px]"
                >
                    SLA {expected}ms
                </text>
            {/if}

            <!-- latency line -->
            {#if linePath}
                <path d={linePath} fill="none" class="stroke-sky-500" stroke-width="1.5" />
            {/if}

            <!-- successful probes -->
            {#each okPoints as p (p.timestamp)}
                <circle
                    cx={x(p.timestamp)}
                    cy={y(p.response_time_ms as number)}
                    r="2"
                    class="fill-sky-500"
                >
                    <title>{formatDateTime(p.timestamp)} — {p.response_time_ms} ms</title>
                </circle>
            {/each}

            <!-- failed probes: red marks on the baseline -->
            {#each failedPoints as p (p.timestamp)}
                <circle cx={x(p.timestamp)} cy={height - pad.bottom} r="3.5" class="fill-rose-500">
                    <title>
                        {formatDateTime(p.timestamp)} — DOWN (see incidents table for details)
                    </title>
                </circle>
            {/each}

            <!-- x labels -->
            <text x={pad.left} y={height - 8} class="fill-muted text-[10px]">
                {formatDateTime(new Date(windowStartMs).toISOString())}
            </text>
            <text
                x={width - pad.right}
                y={height - 8}
                text-anchor="end"
                class="fill-muted text-[10px]"
            >
                now
            </text>
        </svg>
    {/if}
</div>
