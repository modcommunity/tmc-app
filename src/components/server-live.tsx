import { FiUsers, FiWifiOff } from 'react-icons/fi'

import type { ContentSummaryT } from '~/lib/api/contract'
import type { LatencySeriesT, ServerQueryResultT } from '~/lib/ipc/schemas'
import { LatencySparkline, LatencyValue } from './latency-graph'

/**
 * The live strip on a server card: latency, a sparkline, and the player count
 * as measured right now.
 *
 * The distinction this component exists to make visible is **whose number you
 * are looking at**. The website's count is whatever its scanner last recorded,
 * which on a quiet game can be minutes old; the app's is from a datagram this
 * device sent a moment ago. When a live result is in hand it wins and is marked
 * as live; until then the API's figure is shown plainly rather than as a
 * spinner, because a stale-but-real number beats an empty box.
 */

export function LivePlayers({
    item,
    result,
}: {
    item: ContentSummaryT
    result: ServerQueryResultT | undefined
}) {
    /*
     * `players != null`, not `online`.
     *
     * Most of our ~80 server games have no protocol this build speaks, so their
     * probe resolves to `TCP_ONLY`: the connect succeeds — the server IS online
     * and the latency is real — but no player count came back. Treating that as
     * a live count would put the "measured just now" dot next to the website's
     * minutes-old figure, which is the one claim this component exists to avoid
     * making.
     */
    const live = result?.online && result.players != null ? result : undefined

    const current = live?.players ?? item.server?.curUsers ?? 0
    const max = live?.maxPlayers ?? item.server?.maxUsers ?? 0

    return (
        <span
            className="flex items-center gap-1"
            title={live ? 'Measured just now' : 'From the website'}
        >
            <FiUsers className="size-3" />
            <span className="tabular-nums">
                {current}/{max}
            </span>
            {live && (
                <span
                    aria-label="live"
                    className="size-1.5 rounded-full bg-success"
                />
            )}
        </span>
    )
}

/**
 * Which of the four things a row can be saying about its latency.
 *
 * Derived in one place because the inputs are easy to combine wrongly: a row
 * with `probeable: false` has no error and no result, and so is indistinguishable
 * from a row whose first probe is still in flight unless the caller is told
 * which it is.
 */
export function latencyState({
    result,
    error,
    probeable,
    settled,
}: {
    result: ServerQueryResultT | undefined
    error: string | undefined
    /** There is an address to probe at all. */
    probeable: boolean
    /** A probe has resolved for this row, one way or the other. */
    settled: boolean
}): 'measured' | 'timeout' | 'waiting' | 'none' {
    if (!probeable) return 'none'
    if (error || (result && !result.online)) return 'timeout'
    if (result) return 'measured'

    return settled ? 'timeout' : 'waiting'
}

export function LiveLatency({
    result,
    series,
    error,
    probeable = true,
    settled,
    className = '',
}: {
    result: ServerQueryResultT | undefined
    series: LatencySeriesT | undefined
    error: string | undefined
    probeable?: boolean
    /** A probe has resolved for this row — see `latencyState`. */
    settled: boolean
    className?: string
}) {
    const state = latencyState({ result, error, probeable, settled })

    /*
     * A bimodal series means something is answering some probes from a cache,
     * so the fast number is not the round trip a player will get. The row has
     * no space for the explanation — the full sentence is on the view page's
     * graph — so it carries a marker and the tooltip.
     */
    const cached = series?.cache ?? null

    return (
        <span
            className={`flex shrink-0 items-center gap-1 text-[0.7rem] ${className}`}
        >
            {state === 'timeout' && (
                <FiWifiOff className="size-3 shrink-0 text-lat-dead" />
            )}

            <LatencyValue
                rttMs={result?.rttMs}
                state={state}
                title={state === 'timeout' ? (error ?? undefined) : undefined}
            />

            {cached && (
                <span
                    className="text-lat-fair"
                    aria-label="Replies look cached"
                    title={`Replies look cached — ${cached.fastMs}ms on ${Math.round(
                        cached.fastShare * 100
                    )}% of checks and ${cached.slowMs}ms on the rest. Expect closer to ${
                        cached.slowMs
                    }ms in game.`}
                >
                    ~
                </span>
            )}
        </span>
    )
}

/**
 * The card's latency history, sat along the bottom edge.
 *
 * Full-bleed and short: it is a texture, not a chart. Anything worth reading a
 * value off is on the server's own page, where there is room for axes and the
 * summary statistics. Renders nothing until there are two samples, so a card
 * does not reserve space for a line that cannot exist yet.
 */
export function LiveLatencyStrip({
    series,
}: {
    series: LatencySeriesT | undefined
}) {
    if ((series?.samples.length ?? 0) < 2) return null

    return (
        <div className="mt-2 -mb-3 -mx-3 border-t border-border/60 bg-surface-secondary/40 px-3 pb-1 pt-1">
            <LatencySparkline series={series} className="h-6 w-full" />
        </div>
    )
}
