/// <reference types="node" />
import { defineConfig } from 'vitest/config';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
    plugins: [sveltekit(), tailwindcss()],

    server: {
        // dev-mode proxy: the app speaks same-origin (`location.host`), so in
        // `vite dev` forward backend endpoints to the Rust server. The target
        // port follows the backend via BACKEND_PORT (default: 3000).
        proxy: {
            '/ws': {
                target: `http://127.0.0.1:${process.env.BACKEND_PORT ?? 3000}`,
                ws: true
            },
            '/api': {
                target: `http://127.0.0.1:${process.env.BACKEND_PORT ?? 3000}`
            }
        }
    },

    test: {
        include: ['src/**/*.{test,spec}.{js,ts}']
    }
});
