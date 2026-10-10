<script lang="ts">
    import type { ServiceStatus } from '$lib/types';

    let { status, size = 'md' }: { status: ServiceStatus; size?: 'sm' | 'md' } = $props();

    const styles: Record<ServiceStatus, string> = {
        healthy: 'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 ring-emerald-500/30',
        degraded: 'bg-amber-500/15 text-amber-600 dark:text-amber-400 ring-amber-500/30',
        down: 'bg-rose-500/15 text-rose-600 dark:text-rose-400 ring-rose-500/30'
    };
</script>

<span
    class="inline-flex items-center rounded-full font-semibold uppercase tracking-wide ring-1 {styles[
        status
    ]} {size === 'sm' ? 'px-1.5 py-0.5 text-[10px]' : 'px-2 py-0.5 text-xs'}"
>
    {status}
</span>
