import {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
} from 'react'

import { ipc } from '~/lib/ipc/commands'
import { subscribe } from '~/lib/ipc'
import { DownloadSchema, type DownloadT } from '~/lib/ipc/schemas'

/**
 * The download queue, as the UI holds it.
 *
 * PUSH, NOT POLL
 * --------------
 * Rust emits a row on every change, and a running transfer changes twice a
 * second. Four hundred of those polled from React would be eight hundred IPC
 * round trips a second; as an event stream it is one message per changed row,
 * and this provider is where they are coalesced.
 *
 * WHY IT SITS ABOVE THE ROUTER
 * ----------------------------
 * Same reason the live-query registry does. Downloads keep running while
 * somebody browses, and the header's "3 downloading" badge has to be right on
 * every screen, not only on the one that happens to be mounted.
 *
 * THE RENDER SIGNAL
 * -----------------
 * `version` is a counter, and the row map is a `Map` mutated in place behind
 * it. A new `Map` per event would re-render every consumer for a progress tick
 * on one row; bumping a counter lets a card read its own row and lets the
 * badge read a total, with each subscriber re-rendering because it asked to.
 */

type Ctx = {
    /** Every row, in the order Rust sorts them: active, then priority. */
    downloads: DownloadT[]
    byId: (id: string) => DownloadT | undefined
    /** Rows a given sandbox queued, via the `meta` the enqueuer attached. */
    forSandbox: (sandboxId: number) => DownloadT[]
    active: number
    speedBps: number
    limitBps: number | null
    /** Bumped on every change; the render signal. */
    version: number
    refresh: () => Promise<void>
    pause: (id: string) => Promise<void>
    resume: (id: string) => Promise<void>
    cancel: (id: string) => Promise<void>
    setPriority: (id: string, priority: number) => Promise<void>
    setLimit: (id: string, bps: number | null) => Promise<void>
    setGlobalLimit: (bps: number | null) => Promise<void>
    clearFinished: () => Promise<void>
}

const DownloadsContext = createContext<Ctx | null>(null)

/**
 * How often a burst of events is flushed to React, in milliseconds.
 *
 * Rust ticks at 500ms per download; with eight running that is sixteen messages
 * a second, and rendering each one separately is work nobody can see. One flush
 * every 250ms is faster than a progress bar reads as jumpy and turns the worst
 * case into four renders a second regardless of how many transfers there are.
 */
const FLUSH_MS = 250

export function DownloadsProvider({ children }: { children: ReactNode }) {
    const rows = useRef(new Map<string, DownloadT>())
    const pending = useRef(false)

    const [version, setVersion] = useState(0)
    const [limitBps, setLimitBps] = useState<number | null>(null)

    const flush = useCallback(() => {
        if (pending.current) return

        pending.current = true

        window.setTimeout(() => {
            pending.current = false
            setVersion((n) => n + 1)
        }, FLUSH_MS)
    }, [])

    const refresh = useCallback(async () => {
        const snapshot = await ipc.downloadList()

        rows.current = new Map(snapshot.downloads.map((d) => [d.id, d]))

        setLimitBps(snapshot.limitBps)
        setVersion((n) => n + 1)
    }, [])

    // The whole list once, then deltas. The event stream carries changes only,
    // so without this a queue restored from the last session is invisible until
    // something in it happens to move.
    useEffect(() => {
        void refresh().catch(() => {
            /* Not signed in, or the backend is not up yet. */
        })
    }, [refresh])

    useEffect(() => {
        let stop: (() => void) | null = null
        let live = true

        void subscribe(
            'tmc://download',
            DownloadSchema.partial({ id: true }),
            (payload) => {
                // The `idle` event has no id. Nothing to merge; the flush below
                // still lets a consumer notice the queue emptied.
                if (payload.id) rows.current.set(payload.id, payload as DownloadT)

                flush()
            }
        )
            .then((unlisten) => {
                if (live) stop = unlisten
                else unlisten()
            })
            .catch((err) => console.error('[downloads] could not subscribe', err))

        return () => {
            live = false
            stop?.()
        }
    }, [flush])

    const value = useMemo<Ctx>(() => {
        const all = Array.from(rows.current.values())

        // Rust's own ordering: active first, then priority, then arrival. The
        // list on screen is then literally the queue.
        const rank = (status: DownloadT['status']) =>
            status === 'running'
                ? 0
                : status === 'queued'
                  ? 1
                  : status === 'paused'
                    ? 2
                    : status === 'failed'
                      ? 3
                      : status === 'done'
                        ? 4
                        : 5

        const downloads = all.sort(
            (a, b) =>
                rank(a.status) - rank(b.status) ||
                b.priority - a.priority ||
                a.queuedAt.localeCompare(b.queuedAt)
        )

        const act = async (fn: () => Promise<unknown>) => {
            await fn()
            // The command's own event will arrive, but refreshing makes the
            // click feel immediate rather than a quarter-second later.
            await refresh()
        }

        return {
            downloads,
            version,
            byId: (id) => rows.current.get(id),
            forSandbox: (sandboxId) =>
                downloads.filter((d) => d.meta.sandboxId === String(sandboxId)),
            active: downloads.filter(
                (d) => d.status === 'running' || d.status === 'queued'
            ).length,
            speedBps: downloads
                .filter((d) => d.status === 'running')
                .reduce((sum, d) => sum + d.speedBps, 0),
            limitBps,
            refresh,
            pause: (id) => act(() => ipc.downloadPause(id)),
            resume: (id) => act(() => ipc.downloadResume(id)),
            cancel: (id) => act(() => ipc.downloadCancel(id)),
            setPriority: (id, priority) =>
                act(() => ipc.downloadSetPriority(id, priority)),
            setLimit: (id, bps) => act(() => ipc.downloadSetLimit(id, bps)),
            setGlobalLimit: async (bps) => {
                await ipc.downloadSetGlobalLimit(bps)
                setLimitBps(bps)
            },
            clearFinished: () => act(() => ipc.downloadClearFinished()),
        }
    }, [version, limitBps, refresh])

    return (
        <DownloadsContext.Provider value={value}>
            {children}
        </DownloadsContext.Provider>
    )
}

export function useDownloads() {
    const ctx = useContext(DownloadsContext)

    if (!ctx)
        throw new Error('useDownloads must be used inside a DownloadsProvider')

    return ctx
}

/** `1.4 GB`, `812 KB`. */
export function formatBytes(bytes: number | null | undefined): string {
    if (bytes == null) return '—'

    const units = ['B', 'KB', 'MB', 'GB', 'TB']

    let value = bytes
    let unit = 0

    while (value >= 1024 && unit < units.length - 1) {
        value /= 1024
        unit += 1
    }

    // No decimals for bytes and kilobytes: `1.0 KB` is noise, and the number is
    // already precise enough to be useless at that scale.
    return `${value.toFixed(unit >= 2 ? 1 : 0)} ${units[unit]}`
}

/** `4.2 MB/s`. */
export function formatSpeed(bps: number): string {
    if (bps <= 0) return '—'

    return `${formatBytes(bps)}/s`
}

/**
 * `2m 14s`, `1h 03m`.
 *
 * Two units at most. An ETA is a guess, and `1h 03m 12s` claims a precision the
 * number does not have.
 */
export function formatEta(seconds: number | null): string {
    if (seconds == null || seconds <= 0) return '—'

    if (seconds < 60) return `${Math.round(seconds)}s`

    const minutes = Math.floor(seconds / 60)
    const rest = Math.round(seconds % 60)

    if (minutes < 60) return `${minutes}m ${String(rest).padStart(2, '0')}s`

    const hours = Math.floor(minutes / 60)

    return `${hours}h ${String(minutes % 60).padStart(2, '0')}m`
}
