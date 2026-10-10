<script lang="ts">
    import { formatRelative } from '$lib/format';
    import type { AlertPoint } from '$lib/types';
    import ServiceBadge from './ServiceBadge.svelte';

    let {
        points,
        busyId = null,
        testingId = null,
        ontoggle,
        ontest,
        onedit,
        ondelete
    }: {
        points: AlertPoint[];
        busyId?: string | null;
        testingId?: string | null;
        ontoggle: (point: AlertPoint, enabled: boolean) => void;
        ontest: (point: AlertPoint) => void;
        onedit: (point: AlertPoint) => void;
        ondelete: (point: AlertPoint) => void;
    } = $props();

    let now = $state(Date.now());
    // Refresh the relative "last check" labels periodically.
    setInterval(() => (now = Date.now()), 30_000);

    const isBusy = (id: string) => busyId === id || testingId === id;
</script>

<div class="overflow-x-auto rounded-lg border border-line">
    <table class="w-full min-w-3xl text-left text-sm">
        <thead class="bg-surface-2 text-xs uppercase tracking-wide text-muted">
            <tr>
                <th class="px-4 py-3">Name</th>
                <th class="px-4 py-3">Type</th>
                <th class="px-4 py-3">Interval</th>
                <th class="px-4 py-3">SLA</th>
                <th class="px-4 py-3">Last check</th>
                <th class="px-4 py-3">Enabled</th>
                <th class="px-4 py-3 text-right">Actions</th>
            </tr>
        </thead>
        <tbody>
            {#each points as point (point.id)}
                <tr
                    class="border-t border-line hover:bg-surface-2/50 {isBusy(point.id)
                        ? 'opacity-60'
                        : ''}"
                >
                    <td class="max-w-72 px-4 py-3">
                        <div class="truncate font-medium text-text" title={point.name}>
                            {point.name}
                        </div>
                        <a
                            href={point.url}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="block truncate font-mono text-xs text-muted hover:text-text hover:underline"
                            title={point.url}
                        >
                            {point.url}
                        </a>
                    </td>
                    <td class="px-4 py-3"><ServiceBadge type={point.service_type} size="sm" /></td>
                    <td class="px-4 py-3 font-mono text-xs">{point.check_interval_seconds}s</td>
                    <td class="px-4 py-3 font-mono text-xs">{point.expected_response_time_ms}ms</td>
                    <td class="px-4 py-3 text-xs text-muted">
                        {point.last_checked_at
                            ? formatRelative(point.last_checked_at, now)
                            : 'never'}
                    </td>
                    <td class="px-4 py-3">
                        <button
                            type="button"
                            role="switch"
                            aria-checked={point.enabled}
                            aria-label="Toggle enabled"
                            disabled={isBusy(point.id)}
                            onclick={() => ontoggle(point, !point.enabled)}
                            class="relative h-5 w-9 rounded-full transition-colors {point.enabled
                                ? 'bg-emerald-500'
                                : 'bg-slate-400 dark:bg-slate-600'} disabled:opacity-50"
                        >
                            <span
                                class="absolute top-0.5 h-4 w-4 rounded-full bg-white transition-all {point.enabled
                                    ? 'left-4.5'
                                    : 'left-0.5'}"
                            ></span>
                        </button>
                    </td>
                    <td class="px-4 py-3">
                        <div class="flex justify-end gap-1">
                            <button
                                type="button"
                                disabled={isBusy(point.id)}
                                onclick={() => ontest(point)}
                                class="rounded-md border border-line px-2 py-1 text-xs hover:bg-surface-2 disabled:opacity-50"
                            >
                                {testingId === point.id ? 'Testing…' : 'Test now'}
                            </button>
                            <button
                                type="button"
                                disabled={isBusy(point.id)}
                                onclick={() => onedit(point)}
                                class="rounded-md border border-line px-2 py-1 text-xs hover:bg-surface-2 disabled:opacity-50"
                            >
                                Edit
                            </button>
                            <button
                                type="button"
                                disabled={isBusy(point.id)}
                                onclick={() => ondelete(point)}
                                class="rounded-md border border-rose-500/40 px-2 py-1 text-xs text-rose-600 hover:bg-rose-500/10 disabled:opacity-50 dark:text-rose-400"
                            >
                                Delete
                            </button>
                        </div>
                    </td>
                </tr>
            {/each}
        </tbody>
    </table>
</div>
