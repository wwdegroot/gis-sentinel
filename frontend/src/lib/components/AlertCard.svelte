<script lang="ts">
    import { onMount } from 'svelte';
    import { formatDuration } from '$lib/format';
    import type { AlertEvent } from '$lib/types';
    import LatencyMeter from './LatencyMeter.svelte';
    import ServiceBadge from './ServiceBadge.svelte';
    import StatusBadge from './StatusBadge.svelte';

    let { alert }: { alert: AlertEvent } = $props();

    // Ticking clock for the live "down for …" duration.
    let now = $state(Date.now());
    onMount(() => {
        const timer = setInterval(() => (now = Date.now()), 1000);
        return () => clearInterval(timer);
    });

    const duration = $derived(formatDuration(Math.max(0, now - Date.parse(alert.triggered_at))));
    const accent = $derived(
        alert.status === 'down'
            ? 'border-l-rose-500'
            : alert.status === 'degraded'
              ? 'border-l-amber-500'
              : 'border-l-emerald-500'
    );
</script>

<div
    class="flex flex-col gap-3 rounded-lg border border-line border-l-4 bg-surface p-4 shadow-sm {accent}"
>
    <div class="flex items-start justify-between gap-2">
        <div class="min-w-0">
            <div class="truncate font-semibold text-text" title={alert.name}>{alert.name}</div>
            <a
                href={alert.url}
                target="_blank"
                rel="noopener noreferrer"
                class="block max-w-64 truncate text-xs text-muted hover:text-text hover:underline sm:max-w-80"
                title={alert.url}
            >
                {alert.url}
            </a>
        </div>
        <StatusBadge status={alert.status} />
    </div>

    <div class="flex flex-wrap items-center gap-2">
        <ServiceBadge type={alert.service_type} />
        <span class="text-xs text-muted">
            {alert.status === 'healthy' ? 'observed for' : 'ongoing for'}
            <span class="font-mono font-medium text-text">{duration}</span>
        </span>
    </div>

    <LatencyMeter
        response_time_ms={alert.response_time_ms}
        expected={alert.expected_response_time_ms}
    />

    <div class="border-l-2 border-line pl-2 text-sm text-text/90">{alert.reason}</div>
</div>
