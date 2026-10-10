<script lang="ts">
    import { createAlertPoint, updateAlertPoint, ApiError } from '$lib/api';
    import type { AlertPoint, ServiceType } from '$lib/types';
    import {
        emptyForm,
        formToPayload,
        pointToForm,
        validateForm,
        type AlertPointForm
    } from '$lib/validation';

    let {
        point = null,
        onClose,
        onSaved
    }: {
        /** `null` = create mode, otherwise edit this point. */
        point: AlertPoint | null;
        onClose: () => void;
        onSaved: (saved: AlertPoint) => void;
    } = $props();

    const serviceTypes: ServiceType[] = ['WMS', 'WFS', 'WMTS', 'OAF', 'ArcGIS_REST', 'HTTP'];

    // The form is initialized once from the prop (the modal is remounted for
    // every open); $state.snapshot detaches from the parent's proxy.
    // svelte-ignore state_referenced_locally
    const initialPoint: AlertPoint | null = point === null ? null : $state.snapshot(point);
    // svelte-ignore state_referenced_locally
    let form: AlertPointForm = $state(initialPoint ? pointToForm(initialPoint) : emptyForm());
    let errors = $state<Record<string, string>>({});
    let saving = $state(false);
    let serverError = $state('');

    const isEdit = initialPoint !== null;
    const title = $derived(isEdit ? `Edit “${initialPoint?.name}”` : 'New monitoring point');

    function addHeaderRow() {
        form.headers = [...form.headers, { name: '', value: '' }];
    }

    function removeHeaderRow(index: number) {
        form.headers = form.headers.filter((_, i) => i !== index);
    }

    async function save(event: SubmitEvent) {
        event.preventDefault();
        serverError = '';

        const result = formToPayload(form);
        if (!result.payload) {
            errors = result.errors;
            return;
        }
        errors = {};
        saving = true;
        try {
            const saved = isEdit
                ? await updateAlertPoint(initialPoint!.id, result.payload)
                : await createAlertPoint(result.payload);
            onSaved(saved);
        } catch (e) {
            if (e instanceof ApiError) {
                serverError = e.message;
                if (e.details) errors = { ...errors, ...e.details };
            } else {
                serverError = e instanceof Error ? e.message : String(e);
            }
        } finally {
            saving = false;
        }
    }

    /** Live re-validation once a field was marked invalid. */
    function revalidate() {
        if (Object.keys(errors).length > 0) errors = validateForm(form);
    }
</script>

<div class="fixed inset-0 z-50 overflow-y-auto bg-black/60 p-4" role="dialog" aria-modal="true">
    <div
        class="mx-auto my-8 w-full max-w-2xl rounded-xl border border-line bg-surface p-6 shadow-xl"
    >
        <div class="mb-4 flex items-center justify-between">
            <h2 class="text-lg font-bold">{title}</h2>
            <button
                type="button"
                onclick={onClose}
                aria-label="Close"
                class="rounded-md p-1 text-muted hover:bg-surface-2 hover:text-text"
            >
                ✕
            </button>
        </div>

        <form class="flex flex-col gap-4" onsubmit={save}>
            {#if serverError}
                <div
                    class="rounded-md bg-rose-500/10 px-3 py-2 text-sm text-rose-600 dark:text-rose-400"
                >
                    {serverError}
                </div>
            {/if}

            <div class="grid gap-4 sm:grid-cols-2">
                <label class="flex flex-col gap-1 text-sm">
                    <span class="font-medium">Name</span>
                    <input
                        type="text"
                        bind:value={form.name}
                        oninput={revalidate}
                        class="rounded-md border border-line bg-surface px-3 py-2 {errors.name
                            ? 'border-rose-500'
                            : ''}"
                    />
                    {#if errors.name}<span class="text-xs text-rose-500">{errors.name}</span>{/if}
                </label>

                <label class="flex flex-col gap-1 text-sm">
                    <span class="font-medium">Service type</span>
                    <select
                        bind:value={form.service_type}
                        class="rounded-md border border-line bg-surface px-3 py-2"
                    >
                        {#each serviceTypes as type (type)}
                            <option value={type}>{type.replace(/_/g, ' ')}</option>
                        {/each}
                    </select>
                </label>
            </div>

            <label class="flex flex-col gap-1 text-sm">
                <span class="font-medium">Endpoint URL</span>
                <input
                    type="text"
                    placeholder="https://host/geoserver/wms"
                    bind:value={form.url}
                    oninput={revalidate}
                    class="rounded-md border border-line bg-surface px-3 py-2 font-mono text-xs {errors.url
                        ? 'border-rose-500'
                        : ''}"
                />
                {#if errors.url}<span class="text-xs text-rose-500">{errors.url}</span>{/if}
            </label>

            <div class="grid gap-4 sm:grid-cols-3">
                <label class="flex flex-col gap-1 text-sm">
                    <span class="font-medium">Check interval (s)</span>
                    <input
                        type="number"
                        min="10"
                        max="86400"
                        bind:value={form.check_interval_seconds}
                        oninput={revalidate}
                        class="rounded-md border border-line bg-surface px-3 py-2 {errors.check_interval_seconds
                            ? 'border-rose-500'
                            : ''}"
                    />
                    {#if errors.check_interval_seconds}
                        <span class="text-xs text-rose-500">{errors.check_interval_seconds}</span>
                    {/if}
                </label>

                <label class="flex flex-col gap-1 text-sm">
                    <span class="font-medium">Expected SLA (ms)</span>
                    <input
                        type="number"
                        min="1"
                        max="600000"
                        bind:value={form.expected_response_time_ms}
                        oninput={revalidate}
                        class="rounded-md border border-line bg-surface px-3 py-2 {errors.expected_response_time_ms
                            ? 'border-rose-500'
                            : ''}"
                    />
                    {#if errors.expected_response_time_ms}
                        <span class="text-xs text-rose-500">{errors.expected_response_time_ms}</span
                        >
                    {/if}
                </label>

                <label class="flex flex-col gap-1 text-sm">
                    <span class="font-medium">HTTP method</span>
                    <select
                        bind:value={form.http_method}
                        class="rounded-md border border-line bg-surface px-3 py-2"
                    >
                        <option value="GET">GET</option>
                        <option value="POST">POST</option>
                    </select>
                </label>
            </div>

            <!-- custom headers -->
            <div class="flex flex-col gap-2">
                <div class="flex items-center justify-between">
                    <span class="text-sm font-medium">Custom headers</span>
                    <button
                        type="button"
                        onclick={addHeaderRow}
                        class="rounded-md border border-line px-2 py-1 text-xs hover:bg-surface-2"
                    >
                        + Add header
                    </button>
                </div>
                {#if errors.headers}
                    <span class="text-xs text-rose-500">{errors.headers}</span>
                {/if}
                {#each form.headers as row, i}
                    <div class="flex items-center gap-2">
                        <input
                            type="text"
                            placeholder="X-Api-Key"
                            bind:value={row.name}
                            class="w-40 rounded-md border border-line bg-surface px-2 py-1.5 font-mono text-xs"
                        />
                        <input
                            type="text"
                            placeholder="value"
                            bind:value={row.value}
                            class="flex-1 rounded-md border border-line bg-surface px-2 py-1.5 font-mono text-xs"
                        />
                        <button
                            type="button"
                            onclick={() => removeHeaderRow(i)}
                            aria-label="Remove header"
                            class="rounded-md p-1 text-muted hover:bg-surface-2 hover:text-rose-500"
                        >
                            ✕
                        </button>
                    </div>
                {/each}
            </div>

            <!-- auth -->
            <div class="flex flex-col gap-2 rounded-lg border border-line p-3">
                <span class="text-sm font-medium">Authentication</span>
                <select
                    bind:value={form.authKind}
                    class="rounded-md border border-line bg-surface px-2 py-1.5 text-sm"
                >
                    <option value="none">None</option>
                    <option value="basic">HTTP Basic</option>
                    <option value="bearer">Bearer token</option>
                    <option value="header">Custom header</option>
                </select>
                {#if errors.auth}<span class="text-xs text-rose-500">{errors.auth}</span>{/if}

                {#if form.authKind === 'basic'}
                    <div class="grid gap-2 sm:grid-cols-2">
                        <input
                            type="text"
                            placeholder="username"
                            bind:value={form.authUsername}
                            class="rounded-md border border-line bg-surface px-2 py-1.5 text-sm"
                        />
                        <input
                            type="password"
                            placeholder="password"
                            bind:value={form.authPassword}
                            class="rounded-md border border-line bg-surface px-2 py-1.5 text-sm"
                        />
                    </div>
                {:else if form.authKind === 'bearer'}
                    <input
                        type="password"
                        placeholder="token"
                        bind:value={form.authToken}
                        class="rounded-md border border-line bg-surface px-2 py-1.5 text-sm"
                    />
                {:else if form.authKind === 'header'}
                    <div class="grid gap-2 sm:grid-cols-2">
                        <input
                            type="text"
                            placeholder="X-Api-Key"
                            bind:value={form.authHeaderName}
                            class="rounded-md border border-line bg-surface px-2 py-1.5 font-mono text-xs"
                        />
                        <input
                            type="password"
                            placeholder="value"
                            bind:value={form.authHeaderValue}
                            class="rounded-md border border-line bg-surface px-2 py-1.5 font-mono text-xs"
                        />
                    </div>
                {/if}
            </div>

            <label class="flex items-center gap-2 text-sm">
                <input type="checkbox" bind:checked={form.enabled} class="h-4 w-4" />
                Enabled (scheduler probes this point)
            </label>

            <div class="mt-2 flex justify-end gap-2">
                <button
                    type="button"
                    onclick={onClose}
                    class="rounded-md border border-line px-4 py-2 text-sm hover:bg-surface-2"
                >
                    Cancel
                </button>
                <button
                    type="submit"
                    disabled={saving}
                    class="rounded-md bg-sky-600 px-4 py-2 text-sm font-semibold text-white hover:bg-sky-500 disabled:opacity-50"
                >
                    {saving ? 'Saving…' : isEdit ? 'Save changes' : 'Create point'}
                </button>
            </div>
        </form>
    </div>
</div>
