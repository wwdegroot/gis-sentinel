<script lang="ts">
    import type { ServiceType } from '$lib/types';

    let { type, size = 'md' }: { type: ServiceType; size?: 'sm' | 'md' } = $props();

    const label = $derived(type.replace(/_/g, ' '));
</script>

<span
    class="inline-flex items-center rounded-md bg-sky-500/10 font-medium text-sky-600 dark:text-sky-400 ring-1 ring-sky-500/30 {size ===
    'sm'
        ? 'px-1.5 py-0.5 text-[10px]'
        : 'px-2 py-0.5 text-xs'}"
>
    {label}
</span>
