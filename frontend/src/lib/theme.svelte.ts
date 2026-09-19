/** UI theme store (task 3.2): dark by default, persisted in localStorage. */

export type Theme = 'dark' | 'light';

const STORAGE_KEY = 'sentinel-theme';

export class ThemeStore {
    current: Theme = $state('dark');

    constructor() {
        // Apply on construction (SSR is disabled app-wide, but guard anyway).
        if (typeof localStorage !== 'undefined') {
            this.current = localStorage.getItem(STORAGE_KEY) === 'light' ? 'light' : 'dark';
        }
        this.apply();
    }

    toggle(): void {
        this.current = this.current === 'dark' ? 'light' : 'dark';
        this.persist();
        this.apply();
    }

    private persist(): void {
        try {
            localStorage.setItem(STORAGE_KEY, this.current);
        } catch {
            // private mode / storage disabled — theme just won't persist
        }
    }

    private apply(): void {
        if (typeof document === 'undefined') return;
        document.documentElement.classList.toggle('dark', this.current === 'dark');
    }
}

export const theme = new ThemeStore();
