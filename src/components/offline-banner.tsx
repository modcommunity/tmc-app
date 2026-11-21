import { useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { FiRefreshCw, FiWifiOff } from 'react-icons/fi'

import { isIpcError } from '~/lib/ipc'

/** Consecutive network failures before the banner shows; one blip is not an outage. */
const THRESHOLD = 2

/**
 * 'The API is unreachable': shown when the OS says there is no network, or when
 * the last few queries all failed with a network error and none has succeeded
 * since. Cached data keeps rendering underneath; this only says why it is stale.
 */
export default function OfflineBanner() {
    const client = useQueryClient()
    const [online, setOnline] = useState(() => navigator.onLine)
    const [failures, setFailures] = useState(0)

    useEffect(() => {
        const up = () => setOnline(true)
        const down = () => setOnline(false)
        window.addEventListener('online', up)
        window.addEventListener('offline', down)

        return () => {
            window.removeEventListener('online', up)
            window.removeEventListener('offline', down)
        }
    }, [])

    useEffect(
        () =>
            client.getQueryCache().subscribe((event) => {
                if (event.type !== 'updated') return
                const { action } = event

                if (action.type === 'success') setFailures(0)
                else if (
                    action.type === 'error' &&
                    isIpcError(action.error) &&
                    action.error.code === 'network'
                )
                    setFailures((n) => n + 1)
            }),
        [client]
    )

    if (online && failures < THRESHOLD) return null

    return (
        <div
            role="status"
            className="flex shrink-0 items-center gap-2 border-b border-border bg-surface-secondary px-4 py-1.5 text-xs text-foreground"
        >
            <FiWifiOff className="size-3.5 shrink-0 text-accent" aria-hidden />
            <span className="min-w-0 flex-1">
                {online
                    ? 'Can’t reach GameSpice right now. Showing what was saved earlier.'
                    : 'You’re offline. Showing what was saved earlier.'}
            </span>
            <button
                type="button"
                onClick={() => {
                    setFailures(0)
                    void client.invalidateQueries()
                }}
                className="no-drag inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
            >
                <FiRefreshCw className="size-3" aria-hidden />
                Retry
            </button>
        </div>
    )
}
