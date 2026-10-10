<script lang="ts">
    import '../app.css';
    import { onMount } from 'svelte';
    import { goto } from '$app/navigation';
    import { page } from '$app/state';
    import { sentinel } from '$lib/sentinelSocket.svelte';
    import { theme } from '$lib/theme.svelte';
    import { auth } from '$lib/auth.svelte';
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

    const onLoginPage = $derived(page.url.pathname.startsWith('/login'));

    // Resolve the session cookie once on app load (auth plan §9): without a
    // valid session every protected page would 401, so bounce to /login.
    onMount(() => auth.init());

    // Auth gate: any page other than /login requires a logged-in user.
    $effect(() => {
        if (auth.ready && !auth.currentUser && !onLoginPage) {
            const next = page.url.pathname === '/' ? '' : `?redirect=${page.url.pathname}`;
            goto(`/login${next}`);
        }
    });

    // App-wide socket (Q4-b): the WS upgrade is cookie-authenticated, so it
    // only runs while a session exists; `close()` stops the reconnect loop.
    $effect(() => {
        if (auth.currentUser) {
            sentinel.connect();
        } else if (auth.ready) {
            sentinel.close();
        }
    });
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
                {#if auth.currentUser}
                    <span
                        class="hidden max-w-52 truncate text-sm text-muted md:inline"
                        title={auth.currentUser.email}
                    >
                        {auth.currentUser.email}
                        <span
                            class="ml-1 rounded bg-surface-2 px-1.5 py-0.5 text-xs uppercase tracking-wide {auth.isAdmin
                                ? 'text-sky-600 dark:text-sky-400'
                                : ''}"
                        >
                            {auth.currentUser.role}
                        </span>
                    </span>
                    <button
                        type="button"
                        onclick={() => {
                            auth.logout().then(() => goto('/login'));
                        }}
                        class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-surface-2"
                    >
                        Logout
                    </button>
                {/if}
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
