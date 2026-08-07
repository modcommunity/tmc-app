import { FiUsers, FiWifiOff } from 'react-icons/fi'

import type { ContentSummaryT } from '~/lib/api/contract'
import type { LatencySeriesT, ServerQueryResultT } from '~/lib/ipc/schemas'
import { LatencySparkline, latencyTone } from './latency-graph'

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

export function LiveLatency({
    result,
    series,
    error,
    pending,
}: {
    result: ServerQueryResultT | undefined
    series: LatencySeriesT | undefined
    error: string | undefined
    /** No result yet and none refused — a probe is in flight. */
    pending: boolean
}) {
    if (error || (result && !result.online)) {
        return (
            <span
                className="flex shrink-0 items-center gap-1 text-[0.7rem] text-danger"
                title={error ?? 'The server did not answer.'}
            >
                <FiWifiOff className="size-3" />
            </span>
        )
    }

    if (!result) {
        return (
            <span className="shrink-0 text-[0.7rem] text-muted">
                {pending ? '…' : '—'}
            </span>
        )
    }

    const tone = latencyTone(result.rttMs)

    /*
     * A bimodal series means something is answering some probes from a cache,
     * so the fast number is not the round trip a player will get. The row has
     * no space for the explanation — the full sentence is on the view page's
     * graph — so it carries a marker and the tooltip.
     */
    const cached = series?.cache ?? null

    return (
        <span className="flex shrink-0 items-center gap-1">
            <LatencySparkline series={series} />
            <span className={`text-[0.7rem] tabular-nums ${tone.text}`}>
                {result.rttMs}ms
            </span>
            {cached && (
                <span
                    className="text-[0.7rem] text-warning"
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
