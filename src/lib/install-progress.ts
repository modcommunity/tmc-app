/**
 * What the play dialog shows while it downloads a game build: the running
 * download's percentage, or null when none reports a size yet.
 */

import type { DownloadT } from '~/lib/ipc/schemas'

export function installPercent(
    downloads: Pick<DownloadT, 'status' | 'total' | 'done' | 'updatedAt'>[]
): number | null {
    const running = downloads
        .filter((d) => d.status === 'running' && d.total)
        .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))[0]

    if (!running?.total) return null

    return Math.min(100, Math.floor((running.done / running.total) * 100))
}
