<script lang="ts">
    import type { AlertPoint } from '$lib/types';

    let {
        point,
        busy = false,
        onConfirm,
        onCancel
    }: {
        point: AlertPoint;
        busy: boolean;
        onConfirm: () => void;
        onCancel: () => void;
    } = $props();
</script>

<div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
    role="dialog"
    aria-modal="true"
>
    <div class="w-full max-w-md rounded-xl border border-line bg-surface p-6 shadow-xl">
        <h2 class="text-lg font-bold">Delete monitoring point?</h2>
        <p class="mt-2 text-sm text-muted">
            <span class="font-medium text-text">“{point.name}”</span> will be removed along with its probe
            history and alerts. This cannot be undone.
        </p>
        <p class="mt-2 truncate font-mono text-xs text-muted" title={point.url}>{point.url}</p>

        <div class="mt-6 flex justify-end gap-2">
            <button
                type="button"
                onclick={onCancel}
                disabled={busy}
                class="rounded-md border border-line px-4 py-2 text-sm hover:bg-surface-2 disabled:opacity-50"
            >
                Cancel
            </button>
            <button
                type="button"
                onclick={onConfirm}
                disabled={busy}
                class="rounded-md bg-rose-600 px-4 py-2 text-sm font-semibold text-white hover:bg-rose-500 disabled:opacity-50"
            >
                {busy ? 'Deleting…' : 'Delete'}
            </button>
        </div>
    </div>
</div>
