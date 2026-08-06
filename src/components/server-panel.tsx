import { useEffect, useState } from 'react'
import { FiRefreshCw, FiUsers } from 'react-icons/fi'

import type { ContentSummaryT } from '~/lib/api/contract'
import { requestFor, useLiveQuery } from '~/lib/hooks/use-live-query'
import type { ServerQueryResultT } from '~/lib/ipc/schemas'
import { LatencyChart, latencyTone } from './latency-graph'

/**
 * A server's live panel: current state, the latency graph, and who is playing.
 *
 * Unlike the card strip this polls on its own short interval and asks for the
 * player roster, because the user is looking at exactly one server and that is
 * the moment the extra round trip is worth making.
 *
 * The protocol is named in the footer. That is not decoration — when the
 * numbers here disagree with the website's, "queried over A2S from this device"
 * is the explanation, and without it a discrepancy just looks like a bug.
 */

/** Fast enough to feel live on a page the user is watching. */
const REFRESH_MS = 8_000

const PROTOCOL_LABELS: Record<string, string> = {
    A2S: 'Source (A2S)',
    MINECRAFT: 'Minecraft',
    MINECRAFT_SLP: 'Minecraft (legacy)',
    QUAKE3: 'Quake 3',
    GAMESPY1: 'GameSpy v1',
    GAMESPY2: 'GameSpy v2',
    GAMESPY3: 'GameSpy v3',
    SAMP: 'SA-MP',
    FIVEM: 'FiveM',
    TCP_ONLY: 'TCP handshake only',
}

export default function ServerPanel({ item }: { item: ContentSummaryT }) {
    const { queryNow, series: seriesFor, enabled } = useLiveQuery()

    const [result, setResult] = useState<ServerQueryResultT | undefined>()
    const [seriesKey, setSeriesKey] = useState<string | undefined>()
    const [error, setError] = useState<string | undefined>()
    const [busy, setBusy] = useState(false)

    const request = requestFor(item, { wantPlayers: true })

    useEffect(() => {
        if (!enabled || !request) return

        let cancelled = false

        const run = async () => {
            setBusy(true)

            const outcome = await queryNow(request)

            if (cancelled) return

            setResult(outcome?.result)
            setError(outcome?.error)
            setSeriesKey(outcome?.key)
            setBusy(false)
        }

        void run()

        const interval = window.setInterval(() => void run(), REFRESH_MS)

        return () => {
            cancelled = true
            window.clearInterval(interval)
        }
        /*
         * Keyed on the ADDRESS, and on nothing that a query result changes.
         * `queryNow` is stable by construction; putting the context's `series`
         * accessor in here (as this once did) made the effect re-run on its own
         * result and re-query without limit.
         */
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [enabled, request?.host, request?.port, request?.queryPort, queryNow])

    // Read during render rather than mirrored into state: the provider bumps
    // `version` when history lands, which re-renders this component anyway.
    const series = seriesKey ? seriesFor(seriesKey) : undefined

    if (!request)
        return (
            <p className="rounded-xl border border-border bg-surface p-3 text-xs text-muted">
                {item.server?.host
                    ? 'This game has no query protocol configured, so live stats are not available.'
                    : 'This server’s owner has hidden its network details.'}
            </p>
        )

    if (!enabled)
        return (
            <p className="rounded-xl border border-border bg-surface p-3 text-xs text-muted">
                Live queries are turned off in Settings → App.
            </p>
        )

    const online = result?.online ?? false
    const tone = latencyTone(result?.rttMs)

    return (
        <section className="flex flex-col gap-3">
            <div className="flex items-center justify-between gap-2">
                <h2 className="text-sm font-semibold">Live</h2>

                <span className="flex items-center gap-2 text-xs text-muted">
                    <span
                        className={`rounded-full px-2 py-0.5 text-[0.65rem] ${
                            online
                                ? 'bg-success text-success-foreground'
                                : 'bg-danger text-danger-foreground'
                        }`}
                    >
                        {online ? 'Responding' : 'No response'}
                    </span>
                    <FiRefreshCw
                        className={`size-3 ${busy ? 'animate-spin' : 'opacity-40'}`}
                        aria-label={busy ? 'Refreshing' : 'Idle'}
                    />
                </span>
            </div>

            {error && !online && (
                <p className="rounded-lg border border-border bg-surface p-2 text-xs text-muted">
                    {error}
                </p>
            )}

            <LatencyChart series={series} />

            {result && (
                <div className="grid grid-cols-2 gap-2 rounded-xl border border-border bg-surface p-3 text-sm sm:grid-cols-4">
                    <Detail
                        label="Players"
                        value={
                            <span className="flex items-center gap-1 tabular-nums">
                                <FiUsers className="size-3" />
                                {result.players ?? '—'}/{result.maxPlayers ?? '—'}
                            </span>
                        }
                    />
                    {result.bots != null && result.bots > 0 && (
                        <Detail label="Bots" value={String(result.bots)} />
                    )}
                    {result.map && <Detail label="Map" value={result.map} />}
                    {result.version && (
                        <Detail label="Version" value={result.version} />
                    )}
                    <Detail
                        label="Latency"
                        value={
                            <span className={`tabular-nums ${tone.text}`}>
                                {result.rttMs}ms
                            </span>
                        }
                    />
                    {result.password != null && (
                        <Detail
                            label="Password"
                            value={result.password ? 'Required' : 'No'}
                        />
                    )}
                </div>
            )}

            {result && result.playerList.length > 0 && (
                <div>
                    <h3 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-muted">
                        Playing now ({result.playerList.length})
                    </h3>

                    <ul className="flex max-h-64 flex-col divide-y divide-border overflow-y-auto rounded-xl border border-border bg-surface">
                        {result.playerList.map((player, index) => (
                            <li
                                key={`${player.name}-${index}`}
                                className="selectable flex items-center justify-between gap-3 px-3 py-1.5 text-xs"
                            >
                                <span className="min-w-0 truncate">{player.name}</span>

                                <span className="flex shrink-0 gap-3 tabular-nums text-muted">
                                    {player.score != null && (
                                        <span title="Score">{player.score}</span>
                                    )}
                                    {player.ping != null && (
                                        <span title="Their ping">{player.ping}ms</span>
                                    )}
                                    {player.duration != null && (
                                        <span title="Time connected">
                                            {formatDuration(player.duration)}
                                        </span>
                                    )}
                                </span>
                            </li>
                        ))}
                    </ul>
                </div>
            )}

            {result && (
                <p className="text-[0.65rem] text-muted">
                    Queried from this device over{' '}
                    {PROTOCOL_LABELS[result.protocol] ?? result.protocol} on port{' '}
                    {result.queriedPort}.
                </p>
            )}
        </section>
    )
}

function formatDuration(seconds: number): string {
    if (seconds < 60) return `${seconds}s`
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m`

    return `${Math.floor(seconds / 3600)}h${Math.floor((seconds % 3600) / 60)}m`
}

function Detail({ label, value }: { label: string; value: React.ReactNode }) {
    return (
        <div className="flex min-w-0 flex-col">
            <span className="text-[0.65rem] uppercase tracking-wide text-muted">
                {label}
            </span>
            <span className="truncate">{value}</span>
        </div>
    )
}
