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

/**
 * How often the visible set is re-queried, when the user has not said.
 *
 * A second. The app's whole claim over the website is that these numbers are
 * measured now rather than whenever a scanner last passed, and a fifteen-second
 * cadence spent most of its life showing a reading old enough that a server
 * could have filled and emptied inside it. One batch of visible rows per second
 * is a handful of datagrams — the concurrency cap in Rust is what bounds the
 * sockets, not the interval.
 */
const DEFAULT_REFRESH_MS = 1_000

/**
 * The bounds the UI and Rust both hold to.
 *
 * Mirrors `LATENCY_INTERVAL_MS_MIN`/`MAX` in `core/src/settings.rs`, which is
 * the enforcing copy — a settings file edited by hand never reaches this one.
 * Clamped here as well so a stored value from a future build with a wider range
 * cannot turn this timer into a spin.
 */
const MIN_REFRESH_MS = 250
const MAX_REFRESH_MS = 300_000

/**
 * How long a newly-registered row waits before its batch goes out.
 *
 * Long enough that scrolling a screenful into view produces ONE batch rather
 * than one per card, short enough that the number is there by the time the
 * user's eye finishes travelling. Without this the first measurement waited out
 * the whole refresh interval — the warm-up timer fires when the PROVIDER
 * mounts, which is at app start, long before any card exists — so opening the
 * server browser showed a column of ellipses and read as broken. That was worth
 * fifteen seconds of nothing at the old cadence, and it is still the mechanism
 * that makes a slow interval usable: whatever the user sets, a row's FIRST
 * number arrives a quarter of a second after it scrolls into view.
 */
const KICK_MS = 250

/**
 * The latency-history key for a server.
 *
 * Must agree character-for-character with `LatencyStore::key` in Rust, which is
 * `format!("{}:{port}", host.trim().to_ascii_lowercase())`. Rust returns the
 * key on every outcome precisely so this does not have to be computed here —
 * the one caller below is the batch-failure path, where no outcome came back to
 * read it from.
 */
function latencyKey(host: string, port: number): string {
    return `${host.trim().toLowerCase()}:${port}`
}

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
    /**
     * The user's refresh cadence, already clamped.
     *
     * Published so the single-server panel polls on the same setting as the
     * browser rather than on a second constant beside it — one number the user
     * set, one number every live surface honours.
     */
    intervalMs: number
}

const LiveQueryContext = createContext<LiveQueryContextT | null>(null)

export function LiveQueryProvider({ children }: { children: ReactNode }) {
    const { app } = useSettings()
    const enabled = app?.liveLatency ?? true

    const refreshMs = Math.min(
        MAX_REFRESH_MS,
        Math.max(MIN_REFRESH_MS, app?.latencyIntervalMs ?? DEFAULT_REFRESH_MS)
    )

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

    /** A tick was asked for while one was in flight; run again once it lands. */
    const queued = useRef(false)

    /*
     * The tick, reached through a ref.
     *
     * `watch` needs to be able to fire one, and `watch` is in every card's
     * effect dependency list. Depending on `tick` directly would give `watch` a
     * new identity whenever the tick was rebuilt, which tears down and rebuilds
     * fifty IntersectionObservers for nothing.
     */
    const tickRef = useRef<() => void>(() => {})
    const kick = useRef<number | null>(null)

    const scheduleKick = useCallback((delay: number) => {
        if (kick.current !== null) return

        kick.current = window.setTimeout(() => {
            kick.current = null
            tickRef.current()
        }, delay)
    }, [])

    const watch = useCallback(
        (request: QueryRequestT) => {
            leaving.delete(request.id)

            const known = watched.has(request.id)

            watched.set(request.id, request)

            // Only a row the registry has never seen is worth interrupting for.
            // A card scrolling back into view already has its number.
            if (!known) scheduleKick(KICK_MS)
        },
        [watched, leaving, scheduleKick]
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
        if (!enabled) return

        /*
         * A tick asked for during another one is remembered, not dropped. A row
         * that registered while a slow batch was in flight would otherwise wait
         * out the whole refresh interval for its first number — the same defect
         * the kick exists to fix, arriving by a different door.
         */
        if (running.current) {
            queued.current = true

            return
        }

        // Retire anything that has been off screen past the grace window.
        const now = Date.now()

        for (const [id, since] of leaving) {
            if (now - since > UNWATCH_GRACE_MS) {
                leaving.delete(id)
                watched.delete(id)
            }
        }

        /*
         * Rust caps a batch at 64 and DROPS the rest, so the order matters:
         * anything past the cap gets no outcome at all and its row sits on
         * "measuring…" indefinitely. Rows that have never been measured go
         * first, so a fast scroll through a long list always spends the cap on
         * the ones with nothing to show rather than re-measuring rows that
         * already have a number.
         */
        const batch = [...watched.values()].sort((a, b) => {
            const left = results.has(a.id) ? 1 : 0
            const right = results.has(b.id) ? 1 : 0

            return left - right
        })

        if (batch.length < 1) return

        running.current = true

        try {
            const outcomes = await ipc.queryServers(batch)

            applyOutcomes(outcomes)
            await refreshSeries(outcomes.map((o) => o.key))
        } catch (err) {
            console.warn('[live] batch query failed', err)

            /*
             * A failed CALL is settled as a failed PROBE for every row in it.
             *
             * Without this the rows stay in the "measuring…" state for the life
             * of the session, and a broken IPC boundary — a command that is not
             * registered, a schema that has drifted — is indistinguishable on
             * screen from a browser full of slow servers. Every row showing
             * `TO` is at least a symptom that points somewhere.
             */
            applyOutcomes(
                batch.map((request) => ({
                    id: request.id,
                    key: latencyKey(request.host, request.port),
                    error: err instanceof Error ? err.message : 'The query failed.',
                }))
            )
        } finally {
            running.current = false

            if (queued.current) {
                queued.current = false
                scheduleKick(KICK_MS)
            }
        }
    }, [
        enabled,
        watched,
        leaving,
        results,
        applyOutcomes,
        refreshSeries,
        scheduleKick,
    ])

    // Keep the ref pointed at the current tick; `watch` fires through it.
    useEffect(() => {
        tickRef.current = () => void tick()
    }, [tick])

    /*
     * The steady refresh. FIRST measurements do not come from here — a row's
     * first probe is fired by `watch`'s kick, because this provider mounts at
     * app start and its warm-up would long since have run by the time anyone
     * opened the browser.
     */
    useEffect(() => {
        if (!enabled) return

        const interval = window.setInterval(() => void tick(), refreshMs)

        return () => window.clearInterval(interval)
    }, [enabled, refreshMs, tick])

    // A pending kick outlives nothing: the provider unmounting means the app is
    // going away, but a stray timer firing into a torn-down tree is a warning
    // in the console and a confusing one.
    useEffect(
        () => () => {
            if (kick.current !== null) window.clearTimeout(kick.current)
        },
        []
    )

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
        () => ({
            watch,
            unwatch,
            get,
            series,
            queryNow,
            version,
            enabled,
            intervalMs: refreshMs,
        }),
        [watch, unwatch, get, series, queryNow, version, enabled, refreshMs]
    )

    return (
        <LiveQueryContext.Provider value={value}>
            {children}
        </LiveQueryContext.Provider>
    )
}

export function useLiveQuery(): LiveQueryContextT {
    const ctx = useContext(LiveQueryContext)

    if (!ctx)
        throw new Error('useLiveQuery must be used inside <LiveQueryProvider>')

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
        // Carried so Rust can match a Server Live Query PLUGIN to this game
        // when no built-in protocol applies — which is what makes the browser
        // work for every app rather than only the nine with a native parser.
        appId: item.app?.id ?? null,
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
        //
        // `wantPlayers` is one of them: a table row that expands asks for the
        // roster, and without re-registering, the registry would keep querying
        // it with the collapsed row's flags and the player list would go stale
        // the moment the first refresh landed.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [
        enabled,
        request?.id,
        request?.host,
        request?.port,
        request?.wantPlayers,
        watch,
        unwatch,
    ])

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
        /** There is an address to probe: the owner did not hide it. */
        probeable: request !== null,
        /** A probe has resolved for this row, successfully or not. */
        settled: entry !== undefined,
    }
}
