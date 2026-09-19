<script lang="ts">
    import { theme } from '$lib/theme.svelte';

    // Placeholder page; global thresholds/notifications have no backend yet.
</script>

<div class="mx-auto max-w-xl rounded-lg border border-line bg-surface p-6">
    <h1 class="text-xl font-bold">Settings</h1>
    <p class="mt-1 text-sm text-muted">Global thresholds and notifications (planned).</p>

    <div class="mt-6 flex items-center justify-between rounded-lg border border-line p-4">
        <div>
            <div class="font-medium">Theme</div>
            <div class="text-sm text-muted">Currently: {theme.current}</div>
        </div>
        <button
            type="button"
            onclick={() => theme.toggle()}
            class="rounded-md bg-surface-2 px-3 py-1.5 text-sm font-medium hover:opacity-80"
        >
            Switch to {theme.current === 'dark' ? 'light' : 'dark'}
        </button>
    </div>
</div>
