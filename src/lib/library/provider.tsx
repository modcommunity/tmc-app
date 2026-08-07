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
import { isIpcError } from '~/lib/ipc'
import { useAuth } from '~/lib/auth/provider'
import type { LibraryRowT, SyncReportT } from '~/lib/ipc/schemas'

/**
 * The library, and the loop that keeps it in step with the account.
 *
 * WHY A PROVIDER AND NOT A QUERY
 * ------------------------------
 * React Query would cover the reads. What it does not cover is the thing that
 * makes this feature work at all: a subscription created in a BROWSER has to
 * reach this device without anyone touching the app. That needs a timer that
 * runs whether or not a library screen is mounted, so it lives above the
 * router — the same reason `LiveQueryProvider` does.
 *
 * WHY THE TIMER STOPS
 * -------------------
 * Polling a hidden window is a request every minute for a screen nobody is
 * looking at, on a phone whose radio it wakes. The loop pauses on
 * `visibilitychange` and takes one immediate pass on the way back, which is the
 * behaviour a user reads as "it noticed straight away".
 *
 * WHAT THE UI CANNOT DO
 * ---------------------
 * Decide what to install. `librarySync` runs the plan in Rust; this side only
 * asks for a pass and re-reads the result. A frontend that chose what to write
 * to a game folder would be one an injected script could talk into writing
 * something.
 */

/** How often a pass runs while the window is visible. */
const SYNC_INTERVAL_MS = 60_000

type LibraryCtxT = {
    rows: LibraryRowT[]
    loading: boolean
    /** The last sync's summary, for the "synced N seconds ago" line. */
    lastSync: { at: number; report: SyncReportT } | null
    error: string | null
    /** Force a full pass. What the "Sync now" button calls. */
    sync: (full?: boolean) => Promise<void>
    /** Re-read the local database without touching the network. */
    refresh: () => Promise<void>
    install: (id: string) => Promise<void>
    uninstall: (id: string) => Promise<void>
    /** Ids with an operation in flight, so a row can disable its own buttons. */
    busy: ReadonlySet<string>
}

const LibraryCtx = createContext<LibraryCtxT | undefined>(undefined)

export function useLibrary(): LibraryCtxT {
    const ctx = useContext(LibraryCtx)

    if (!ctx) throw new Error('Library context is missing.')

    return ctx
}

function messageOf(err: unknown): string {
    if (isIpcError(err)) return err.message

    return err instanceof Error ? err.message : 'Something went wrong.'
}

export function LibraryProvider({ children }: { children: ReactNode }) {
    const { status } = useAuth()
    const signedIn = status === 'signedIn'

    const [rows, setRows] = useState<LibraryRowT[]>([])
    const [loading, setLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [lastSync, setLastSync] = useState<LibraryCtxT['lastSync']>(null)
    const [busy, setBusy] = useState<Set<string>>(new Set())

    /*
     * A ref as well as state: the interval closure below is created once and
     * would otherwise capture the first `syncing` value forever. The ref is
     * what stops two passes overlapping when one takes longer than the
     * interval — which is the normal case the first time a large library
     * installs.
     */
    const running = useRef(false)

    const refresh = useCallback(async () => {
        try {
            setRows(await ipc.libraryList())
            setError(null)
        } catch (err) {
            setError(messageOf(err))
        }
    }, [])

    const sync = useCallback(
        async (full?: boolean) => {
            if (running.current) return

            running.current = true
            setLoading(true)

            try {
                const report = await ipc.librarySync(full)

                setLastSync({ at: Date.now(), report })
                setError(null)
            } catch (err) {
                setError(messageOf(err))
            } finally {
                running.current = false
                setLoading(false)

                // Always re-read, even after a failure: a pass that threw
                // partway may still have installed several items, and leaving
                // the list stale would hide that.
                await refresh()
            }
        },
        [refresh]
    )

    const withBusy = useCallback(
        async (id: string, run: () => Promise<unknown>) => {
            setBusy((prev) => new Set(prev).add(id))

            try {
                await run()
                setError(null)
            } catch (err) {
                setError(messageOf(err))
            } finally {
                setBusy((prev) => {
                    const next = new Set(prev)
                    next.delete(id)

                    return next
                })

                await refresh()
            }
        },
        [refresh]
    )

    const install = useCallback(
        (id: string) => withBusy(id, () => ipc.libraryInstall(id)),
        [withBusy]
    )

    const uninstall = useCallback(
        (id: string) => withBusy(id, () => ipc.libraryUninstall(id)),
        [withBusy]
    )

    // Read the local database immediately on mount, signed in or not — an
    // offline launch should still show what is installed.
    useEffect(() => {
        void refresh()
    }, [refresh])

    // Sign-in and sign-out both change whose library this is.
    useEffect(() => {
        if (!signedIn) return

        // A launch pass is always FULL: the device may have been off while the
        // user unsubscribed from something on their phone, and that is exactly
        // the case a delta cannot express.
        void sync(true)
    }, [signedIn, sync])

    useEffect(() => {
        if (!signedIn) return

        let timer: number | undefined

        const start = () => {
            stop()
            timer = window.setInterval(() => void sync(), SYNC_INTERVAL_MS)
        }

        const stop = () => {
            if (timer !== undefined) window.clearInterval(timer)
            timer = undefined
        }

        const onVisibility = () => {
            if (document.hidden) {
                stop()

                return
            }

            // One pass on the way back, then resume the timer. Coming back to a
            // window that then waits a full minute before noticing anything
            // reads as broken.
            void sync()
            start()
        }

        if (!document.hidden) start()

        document.addEventListener('visibilitychange', onVisibility)

        return () => {
            stop()
            document.removeEventListener('visibilitychange', onVisibility)
        }
    }, [signedIn, sync])

    const value = useMemo<LibraryCtxT>(
        () => ({
            rows,
            loading,
            lastSync,
            error,
            sync,
            refresh,
            install,
            uninstall,
            busy,
        }),
        [rows, loading, lastSync, error, sync, refresh, install, uninstall, busy]
    )

    return <LibraryCtx.Provider value={value}>{children}</LibraryCtx.Provider>
}
