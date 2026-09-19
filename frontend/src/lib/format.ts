/** Small formatting helpers shared by dashboard/history components. */

/** Compact human duration: "2d 3h", "4h 5m", "12m 30s", "45s". */
export function formatDuration(ms: number): string {
    if (!Number.isFinite(ms) || ms < 0) return '—';
    const totalSeconds = Math.floor(ms / 1000);
    const days = Math.floor(totalSeconds / 86_400);
    const hours = Math.floor((totalSeconds % 86_400) / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;

    if (days > 0) return `${days}d ${hours}h`;
    if (hours > 0) return `${hours}h ${minutes}m`;
    if (minutes > 0) return `${minutes}m ${seconds}s`;
    return `${seconds}s`;
}

/** Locale date-time for ISO strings; returns the raw input when unparsable. */
export function formatDateTime(iso: string): string {
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
}

/** "5 min ago" style relative time (coarse). */
export function formatRelative(iso: string, now: number = Date.now()): string {
    const diff = now - Date.parse(iso);
    if (!Number.isFinite(diff)) return iso;
    if (diff < 60_000) return 'just now';
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} min ago`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} h ago`;
    return `${Math.floor(diff / 86_400_000)} d ago`;
}
