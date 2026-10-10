import { redirect } from '@sveltejs/kit';

/** `/sentinel` now points at the unified dashboard (task 3.3, Q1-a). */
export function load() {
    redirect(307, '/');
}
