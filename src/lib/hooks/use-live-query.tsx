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

import { ipc, type QueryRequestT } from '~/lib/ipc/commands'
import type { LatencySeriesT, QueryOutcomeT } from '~/lib/ipc/schemas'
import type { ContentSummaryT } from '~/lib/api/contract'
import { useSettings } from '~/lib/settings/provider'

/**
 * Live server data for whatever is currently on screen.
 *
 * The website can only show you what its scanner last saw, from wherever the
 * scanner lives. This queries each server **directly from the user's device**,
 * with the game's own protocol, so the player count is current and the latency
 * is the one that player will actually experience.
 *
 * The design is a single registry rather than per-card polling:
 *
 *   * **Cards register and deregister as they scroll.** An `IntersectionObserver`
 *     in each card calls `watch`/`unwatch`; only visible rows are ever queried.
 *   * **One timer, one batch.** Every tick sends the whole visible set to Rust
 *     in a single IPC call, which applies the concurrency cap. Fifty cards each
 *     running their own interval would be fifty timers and fifty round trips.
 *   * **Results are keyed by row id**, so a card re-rendering (or scrolling out
 *     and back) keeps the value it already had instead of flashing to "…".
 */

/** How often the visible set is re-queried. */
const REFRESH_MS = 15_000

/** Grace before a scrolled-away row is dropped, so a flick-scroll does not
 *  discard work that is already in flight. */
const UNWATCH_GRACE_MS = 5_000

/** Cached results kept. Many viewports' worth, but not a whole session's. */
const MAX_CACHED_RESULTS = 400

/**
 * Drop entries until `map` is within `limit`, oldest first.
 *
 * A `Map` iterates in insertion order, and `set` on an existing key does not
 * move it — so for the series cache, where every entry is rewritten in place,
 * insertion order IS age order and taking from the front is correct.
 */
function pruneOldest(map: Map<string, unknown>, limit: number) {
    if (map.size <= limit) return

    let excess = map.size - limit

    for (const key of map.keys()) {
        map.delete(key)

        if (--excess <= 0) break
    }
}

type LiveEntry = {
    outcome: QueryOutcomeT
    /** When this result landed, for the "updated Ns ago" affordance. */
    at: number
}

type LiveQueryContextT = {
    /** Start querying this server while its card is on screen. */
    watch: (request: QueryRequestT) => void
    unwatch: (id: string) => void
    /** Latest result for a row, if any. */
    get: (id: string) => LiveEntry | undefined
    /** Latency history for a row's server. */
    series: (key: string) => LatencySeriesT | undefined
    /** Query one server immediately, outside the registry (the view page). */
    queryNow: (request: QueryRequestT) => Promise<QueryOutcomeT | null>
    /**
     * Bumped each time results land.
     *
     * The accessors above are STABLE — they read from refs — so a consumer that
     * needs to recompute when new data arrives depends on this instead. That
     * separation is not a micro-optimisation: an accessor whose identity
     * changed on every result put `queryNow` and `series` into effect
     * dependency lists that re-ran the effect that produced the result, which
     * is an unbounded query loop against someone's game server.
     */
    version: number
    enabled: boolean
}

const LiveQueryContext = createContext<LiveQueryContextT | null>(null)

export function LiveQueryProvider({ children }: { children: ReactNode }) {
    const { app } = useSettings()
    const enabled = app?.liveLatency ?? true

    /*
     * Refs, not state, for the registry. It changes on every scroll tick and
     * re-rendering the whole tree for that would be far more expensive than the
     * queries themselves. The RESULTS are state, because those must paint.
     */
    const watched = useRef(new Map<string, QueryRequestT>()).current
    const leaving = useRef(new Map<string, number>()).current

    /*
     * Results live in refs, with `version` as the render signal. Holding them
     * in state directly would give the context a new `get`/`series` closure on
     * every batch, and those closures end up in effect dependency lists — see
     * the note on `version` above.
     */
    const results = useRef(new Map<string, LiveEntry>()).current
    const seriesMap = useRef(new Map<string, LatencySeriesT>()).current

    const [version, setVersion] = useState(0)

    /** Guards against a slow tick overlapping the next one. */
    const running = useRef(false)

    const watch = useCallback(
        (request: QueryRequestT) => {
            leaving.delete(request.id)
            watched.set(request.id, request)
        },
        [watched, leaving]
    )

    const unwatch = useCallback(
        (id: string) => {
            // Marked, not removed. A fast scroll would otherwise cancel a row
            // the user is about to scroll back to.
            leaving.set(id, Date.now())
        },
        [leaving]
    )

    const applyOutcomes = useCallback(
        (outcomes: QueryOutcomeT[]) => {
            if (outcomes.length < 1) return

            const at = Date.now()

            for (const outcome of outcomes) results.set(outcome.id, { outcome, at })

            /*
             * Bound both maps. A long scroll through thousands of servers would
             * otherwise accumulate an entry per row for the life of the
             * session — the registry is pruned as cards leave, but the results
             * were not. Oldest-first, keeping well above one viewport.
             */
            pruneOldest(results, MAX_CACHED_RESULTS)
            pruneOldest(seriesMap, MAX_CACHED_RESULTS)

            setVersion((n) => n + 1)
        },
        [results, seriesMap]
    )

    const refreshSeries = useCallback(
        async (keys: string[]) => {
            if (keys.length < 1) return

            try {
                const rows = await ipc.latencySeries(keys)

                for (const row of rows) seriesMap.set(row.key, row)

                setVersion((n) => n + 1)
            } catch {
                // History is decoration. A failure must not disturb the grid.
            }
        },
        [seriesMap]
    )

    const tick = useCallback(async () => {
        if (running.current || !enabled) return

        // Retire anything that has been off screen past the grace window.
        const now = Date.now()

        for (const [id, since] of leaving) {
            if (now - since > UNWATCH_GRACE_MS) {
                leaving.delete(id)
                watched.delete(id)
            }
        }

        const batch = [...watched.values()]

        if (batch.length < 1) return

        running.current = true

        try {
            const outcomes = await ipc.queryServers(batch)

            applyOutcomes(outcomes)
            await refreshSeries(outcomes.map((o) => o.key))
        } catch (err) {
            console.warn('[live] batch query failed', err)
        } finally {
            running.current = false
        }
    }, [enabled, watched, leaving, applyOutcomes, refreshSeries])

    /*
     * A short warm-up timer as well as the long one: a card that just scrolled
     * into view should not wait out a full refresh interval for its first
     * number. The tick is cheap when nothing new registered, because Rust
     * short-circuits an empty batch and the grid is capped anyway.
     */
    useEffect(() => {
        if (!enabled) return

        const warm = window.setTimeout(() => void tick(), 300)
        const interval = window.setInterval(() => void tick(), REFRESH_MS)

        return () => {
            window.clearTimeout(warm)
            window.clearInterval(interval)
        }
    }, [enabled, tick])

    // Do not query a backgrounded app; catch up as soon as it returns.
    useEffect(() => {
        const onVisibility = () => {
            if (!document.hidden) void tick()
        }

        document.addEventListener('visibilitychange', onVisibility)

        return () => document.removeEventListener('visibilitychange', onVisibility)
    }, [tick])

    const queryNow = useCallback(
        async (request: QueryRequestT) => {
            try {
                const outcome = await ipc.queryServer(request)

                applyOutcomes([outcome])
                await refreshSeries([outcome.key])

                return outcome
            } catch (err) {
                console.warn('[live] query failed', err)

                return null
            }
        },
        [applyOutcomes, refreshSeries]
    )

    const get = useCallback((id: string) => results.get(id), [results])
    const series = useCallback((key: string) => seriesMap.get(key), [seriesMap])

    const value = useMemo<LiveQueryContextT>(
        () => ({ watch, unwatch, get, series, queryNow, version, enabled }),
        [watch, unwatch, get, series, queryNow, version, enabled]
    )

    return (
        <LiveQueryContext.Provider value={value}>
            {children}
        </LiveQueryContext.Provider>
    )
}

export function useLiveQuery(): LiveQueryContextT {
    const ctx = useContext(LiveQueryContext)

    if (!ctx) throw new Error('useLiveQuery must be used inside <LiveQueryProvider>')

    return ctx
}

/**
 * Turn a browser row into a query request, or `null` when there is nothing to
 * query.
 *
 * Null in three cases, all of which are correct rather than defensive: the
 * item is not a server; the owner hid its network details; or its game declares
 * no query config. In none of those does the app have an address it is entitled
 * to probe.
 */
export function requestFor(
    item: ContentSummaryT,
    opts: { wantPlayers?: boolean } = {}
): QueryRequestT | null {
    const server = item.server

    if (!server?.host || !server.port || !server.query) return null

    return {
        id: `${item.kind}:${item.id}`,
        host: server.host,
        port: server.port,
        queryPort: server.queryPort,
        protocols: server.query.protocols,
        portOffset: server.query.portOffset,
        swapGamePort: server.query.swapGamePort,
        timeoutMs: server.query.timeoutMs,
        wantPlayers: opts.wantPlayers ?? false,
    }
}

/**
 * Register a card with the live registry while it is on screen.
 *
 * Returns the ref to attach and the latest result. The observer has a generous
 * `rootMargin` so a row is queried just before it becomes visible — the number
 * is there when the user's eye arrives, not a second later.
 */
export function useLiveServer<T extends HTMLElement = HTMLElement>(
    request: QueryRequestT | null
) {
    // `version` is unused here beyond forcing a re-render when results land —
    // the accessors are stable, so nothing else would.
    const { watch, unwatch, get, series, enabled } = useLiveQuery()
    const ref = useRef<T | null>(null)

    useEffect(() => {
        if (!enabled || !request) return

        const node = ref.current

        if (!node) return

        const observer = new IntersectionObserver(
            (entries) => {
                if (entries.some((e) => e.isIntersecting)) watch(request)
                else unwatch(request.id)
            },
            { rootMargin: '200px' }
        )

        observer.observe(node)

        return () => {
            observer.disconnect()
            unwatch(request.id)
        }
        // `request` is rebuilt on each render; its IDENTITY changing must not
        // re-subscribe, so the effect keys on the fields that matter.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [enabled, request?.id, request?.host, request?.port, watch, unwatch])

    const entry = request ? get(request.id) : undefined
    const key = entry?.outcome.key

    return {
        ref,
        result: entry?.outcome.result,
        error: entry?.outcome.error,
        updatedAt: entry?.at,
        series: key ? series(key) : undefined,
        /** False when the user turned live queries off — nothing is coming. */
        enabled,
    }
}
