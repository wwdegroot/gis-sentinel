<script lang="ts">
    import '../app.css';
    import { onMount } from 'svelte';
    import { page } from '$app/state';
    import { sentinel } from '$lib/sentinelSocket.svelte';
    import { theme } from '$lib/theme.svelte';
    import ConnectionBadge from '$lib/components/ConnectionBadge.svelte';

    let { children } = $props();

    const links = [
        { href: '/', label: 'Dashboard' },
        { href: '/services', label: 'Services' },
        { href: '/history', label: 'History' },
        { href: '/settings', label: 'Settings' }
    ];

    const isActive = (href: string) =>
        href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);

    // App-wide socket (Q4-b): connect once in the layout so the connection
    // badge is live on every page; the reconnect loop makes this safe.
    onMount(() => sentinel.connect());
</script>

<div class="min-h-screen bg-surface text-text">
    <nav class="border-b border-line bg-surface">
        <div class="container mx-auto flex h-14 items-center justify-between gap-4">
            <div class="flex items-center gap-6">
                <a href="/" class="flex items-center gap-2 font-bold tracking-tight">
                    <span aria-hidden="true">🛰️</span>
                    GIS Sentinel
                </a>
                <div class="hidden gap-1 sm:flex">
                    {#each links as link (link.href)}
                        <a
                            href={link.href}
                            aria-current={isActive(link.href) ? 'page' : undefined}
                            class="rounded-md px-3 py-1.5 text-sm transition-colors {isActive(
                                link.href
                            )
                                ? 'bg-surface-2 font-medium text-text'
                                : 'text-muted hover:bg-surface-2 hover:text-text'}"
                        >
                            {link.label}
                        </a>
                    {/each}
                </div>
            </div>
            <div class="flex items-center gap-3">
                <ConnectionBadge />
                <button
                    type="button"
                    onclick={() => theme.toggle()}
                    aria-label="Toggle dark/light theme"
                    class="rounded-md p-2 text-sm transition-colors hover:bg-surface-2"
                >
                    {theme.current === 'dark' ? '☀️' : '🌙'}
                </button>
            </div>
        </div>
        <!-- mobile nav row -->
        <div class="container mx-auto flex gap-1 pb-2 sm:hidden">
            {#each links as link (link.href)}
                <a
                    href={link.href}
                    aria-current={isActive(link.href) ? 'page' : undefined}
                    class="rounded-md px-3 py-1.5 text-sm transition-colors {isActive(link.href)
                        ? 'bg-surface-2 font-medium text-text'
                        : 'text-muted hover:bg-surface-2 hover:text-text'}"
                >
                    {link.label}
                </a>
            {/each}
        </div>
    </nav>

    <main class="container mx-auto py-6">
        {@render children()}
    </main>
</div>
