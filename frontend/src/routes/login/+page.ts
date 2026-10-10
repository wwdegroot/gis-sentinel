// `/login` itself must never redirect to `/login` (auth gate in the layout);
// the layout checks `page.url.pathname` before gating. The page itself is a
// pure client-side form (SSR disabled app-wide).
export const ssr = false;
export const prerender = true;
