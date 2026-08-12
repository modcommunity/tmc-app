import { useEffect, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import {
    FiChevronDown,
    FiChevronRight,
    FiChevronUp,
    FiLock,
    FiShield,
} from 'react-icons/fi'

import type { BrowseSortT, ContentSummaryT } from '~/lib/api/contract'
import { appLabel } from '~/lib/api/labels'
import { requestFor, useLiveQuery, useLiveServer } from '~/lib/hooks/use-live-query'
import { LatencyChart, LatencyValue } from './latency-graph'
import { latencyState } from './server-live'

/**
 * The server browser as a table.
 *
 * This is the default for servers, and the grid is the alternative — the
 * reverse of every other kind. A server is a row of comparable numbers: ping,
 * players, map, version. Cards put each of those in a different place on the
 * screen, so comparing forty of them means forty separate reads; a table puts
 * each number in a column, and the eye can run down it.
 *
 * A row carries NO graph. Sixty-point sparklines in a scrolling table are
 * visual noise in the one view whose whole purpose is comparing single numbers,
 * and they would fight the column alignment that makes the table worth having.
 * The history is one click away instead: a row expands downward and the chart
 * appears beneath its own data, still inside the same `<tr>`.
 *
 * Several rows may be open at once, and the panel is sized for that: a short
 * chart, so an open row costs a couple of collapsed rows' worth of height
 * rather than a screenful. Comparing two servers' histories is the reason to
 * open one at all, and it cannot be done a row at a time.
 */

/** Columns dropped on a narrow window, in the order they stop earning space. */
const WIDE_ONLY = 'hidden lg:table-cell'
const MEDIUM_ONLY = 'hidden md:table-cell'

/**
 * Colour a player count by how full the server is.
 *
 * The website's `usePlayerColors`. An empty server greys out so it stops
 * competing with the rows that have people on them, and a nearly-full one warns
 * before the user spends thirty seconds connecting to a queue.
 */
function fillTone(players: number, max: number): string {
    if (players < 1) return 'text-muted'
    if (max < 1) return 'text-foreground'

    const share = players / max

    if (share >= 0.95) return 'text-lat-bad'
    if (share >= 0.75) return 'text-lat-fair'

    return 'text-lat-good'
}

/**
 * A column header that sorts.
 *
 * Mirrors the website's `SortableTh`: clicking a new column sorts by it in its
 * natural direction, clicking the active one flips the direction. The natural
 * direction differs per column and matters — names want A→Z, player counts want
 * most-first, and defaulting everything to `desc` puts "Zulu" at the top of an
 * alphabetical sort.
 *
 * Ping is deliberately NOT sortable here. It is measured on this device rather
 * than stored, so there is no server-side ordering to ask for — the "Sort by
 * ping" control beside the sort dropdown reorders what has already been
 * measured, which is a different operation and is labelled as one.
 */
function SortableTh({
    label,
    sortKey,
    sort,
    dir,
    onSort,
    align = 'left',
    className = '',
    naturalDir = 'desc',
}: {
    label: string
    sortKey: BrowseSortT
    sort: BrowseSortT
    dir: 'asc' | 'desc'
    onSort: (key: BrowseSortT, dir: 'asc' | 'desc') => void
    align?: 'left' | 'right'
    className?: string
    naturalDir?: 'asc' | 'desc'
}) {
    const active = sort === sortKey

    return (
        <th className={`p-0 font-medium ${className}`}>
            <button
                type="button"
                onClick={() =>
                    onSort(
                        sortKey,
                        active ? (dir === 'asc' ? 'desc' : 'asc') : naturalDir
                    )
                }
                aria-sort={
                    active ? (dir === 'asc' ? 'ascending' : 'descending') : 'none'
                }
                className={`flex w-full items-center gap-1 px-2 py-2 uppercase tracking-wide transition-colors hover:text-foreground ${
                    align === 'right' ? 'justify-end' : ''
                } ${active ? 'text-foreground' : ''}`}
            >
                {label}
                {active &&
                    (dir === 'asc' ? (
                        <FiChevronUp className="size-3" />
                    ) : (
                        <FiChevronDown className="size-3" />
                    ))}
            </button>
        </th>
    )
}

export default function ServerTable({
    items,
    sort,
    sortDir,
    onSort,
}: {
    items: ContentSummaryT[]
    sort: BrowseSortT
    sortDir: 'asc' | 'desc'
    onSort: (key: BrowseSortT, dir: 'asc' | 'desc') => void
}) {
    /*
     * Any number of rows may stand open, by id.
     *
     * This was an accordion, on the reasoning that an open row is tall enough
     * that several turn the table back into the card list it replaces. That
     * reasoning applies to the panel's height, not to how many may be open —
     * and the thing people actually want from these graphs is to compare two
     * servers' histories, which an accordion makes impossible: opening the
     * second closes the first. The panel is short enough now that a handful
     * open at once still reads as a table.
     */
    const [expanded, setExpanded] = useState<ReadonlySet<string>>(
        () => new Set<string>()
    )

    const toggle = (id: string) =>
        setExpanded((open) => {
            const next = new Set(open)

            if (!next.delete(id)) next.add(id)

            return next
        })

    const sortable = { sort, dir: sortDir, onSort }

    return (
        <div className="overflow-x-auto rounded-xl border border-border">
            <table className="w-full min-w-[56rem] border-collapse text-sm">
                <thead>
                    <tr className="border-b border-border bg-surface-secondary text-left text-xs uppercase tracking-wide text-muted">
                        <th className="w-6 py-2 pl-2" />

                        {/*
                            Ping leads, exactly as it does on the website.

                            It is the column this app exists to provide and the
                            one a player decides on, so it must never be the
                            thing that scrolls out of view — which is precisely
                            what happened when it sat on the right of eight
                            columns in a pane narrower than the table.
                        */}
                        <th className="px-2 py-2 pr-5 text-right font-medium">
                            Ping
                        </th>

                        {/* Name takes the slack (`w-full`) so every numeric
                            column stays exactly as wide as its content — the
                            same trick the website's table uses. */}
                        <SortableTh
                            {...sortable}
                            label="Server"
                            sortKey="name"
                            naturalDir="asc"
                            className="w-full min-w-64"
                        />
                        <th className={`${MEDIUM_ONLY} px-2 py-2 font-medium`}>
                            Country
                        </th>
                        {/* `players`, not `curUsers`: same ordering, but it
                            is the value the default sends and the one every
                            deployed server understands. Sending the newer
                            spelling here would leave the header inactive on the
                            default view and 400 against an older server. */}
                        <SortableTh
                            {...sortable}
                            label="Players"
                            sortKey="players"
                            align="right"
                        />
                        <SortableTh
                            {...sortable}
                            label="Avg"
                            sortKey="avgUsers"
                            align="right"
                            className={MEDIUM_ONLY}
                        />
                        <SortableTh
                            {...sortable}
                            label="Map"
                            sortKey="map"
                            naturalDir="asc"
                            className={`${MEDIUM_ONLY} min-w-28`}
                        />
                        <th
                            className={`${WIDE_ONLY} min-w-36 px-2 py-2 font-medium`}
                        >
                            Host
                        </th>
                    </tr>
                </thead>

                <tbody>
                    {items.map((item) => (
                        <ServerRow
                            key={`${item.kind}-${item.id}`}
                            item={item}
                            expanded={expanded.has(item.id)}
                            onToggle={() => toggle(item.id)}
                        />
                    ))}
                </tbody>
            </table>
        </div>
    )
}

function ServerRow({
    item,
    expanded,
    onToggle,
}: {
    item: ContentSummaryT
    expanded: boolean
    onToggle: () => void
}) {
    const navigate = useNavigate()

    /*
     * The roster is asked for only while the row is OPEN.
     *
     * `wantPlayers` is a second round trip per server on most protocols, which
     * a forty-row table does not want to pay forty times for data no column
     * shows. The expanded panel is the only place it is read.
     */
    const request = requestFor(item, { wantPlayers: expanded })
    const live = useLiveServer<HTMLTableRowElement>(request)

    const result = live.result
    const online = live.error
        ? false
        : (result?.online ?? item.server?.online ?? false)

    const players = result?.players ?? item.server?.curUsers ?? 0
    const measured = result?.players != null

    /*
     * The live slot count is only believed when it is believable.
     *
     * Several games report `max_players` over A2S as something other than the
     * server's capacity — Rust in particular answers with a figure that ignores
     * the queue, so a busy server came back as `144/51`. More players than
     * slots is self-evidently not a capacity, so the API's own `maxUsers` (the
     * number the website shows, taken from the server's config) wins whenever
     * the measured one fails that check.
     *
     * Deliberately only the SLOT count. The measured player count stays
     * authoritative — it is the number this app exists to provide, and it is
     * the one the live dot is claiming.
     */
    const apiMax = item.server?.maxUsers ?? 0
    const liveMax = result?.maxPlayers ?? null
    const max = liveMax != null && liveMax >= players ? liveMax : apiMax

    const state = latencyState({
        result,
        error: live.error,
        probeable: live.probeable,
        settled: live.settled,
    })

    return (
        <>
            <tr
                ref={live.ref}
                onClick={onToggle}
                className={`cursor-pointer border-b border-border transition-colors hover:bg-surface-hover ${
                    expanded ? 'bg-surface-hover' : ''
                }`}
            >
                <td className="py-2 pl-2 align-middle text-muted">
                    {expanded ? (
                        <FiChevronDown className="size-4" aria-label="Collapse" />
                    ) : (
                        <FiChevronRight className="size-4" aria-label="Expand" />
                    )}
                </td>

                <td className="whitespace-nowrap px-2 py-2 pr-5 text-right align-middle">
                    <LatencyValue
                        rttMs={result?.rttMs}
                        state={state}
                        title={
                            state === 'timeout'
                                ? (live.error ?? undefined)
                                : undefined
                        }
                    />
                </td>

                <td className="max-w-0 px-2 py-2 align-middle">
                    <div className="flex items-center gap-2">
                        <span
                            aria-label={online ? 'Online' : 'Offline'}
                            title={online ? 'Online' : 'Offline'}
                            className={`size-2 shrink-0 rounded-full ${
                                online ? 'bg-success' : 'bg-danger'
                            }`}
                        />
                        <span className="truncate font-medium">
                            {result?.name ?? item.name}
                        </span>
                        {item.server?.password && (
                            <FiLock
                                className="size-3 shrink-0 text-muted"
                                aria-label="Password required"
                            />
                        )}
                        {item.server?.secure && (
                            <FiShield
                                className="size-3 shrink-0 text-muted"
                                aria-label="Anti-cheat enabled"
                            />
                        )}
                    </div>
                    {item.app && (
                        <span className="truncate text-xs text-accent">
                            {appLabel(item.app)}
                        </span>
                    )}
                </td>

                <td
                    className={`${MEDIUM_ONLY} max-w-32 truncate px-2 py-2 text-xs text-muted`}
                    title={item.server?.country ?? undefined}
                >
                    {item.server?.country ?? '—'}
                </td>

                <td
                    className="whitespace-nowrap px-2 py-2 text-right tabular-nums"
                    title={measured ? 'Measured just now' : 'From the website'}
                >
                    {/* The count is coloured by how full the server is, which
                        is the website's `usePlayerColors`: a 30/32 reads as
                        "nearly full" at a glance, and an empty one greys out
                        rather than competing with the rows that have players. */}
                    <span className={fillTone(players, max)}>{players}</span>
                    <span className="text-muted">/{max}</span>
                    {measured && (
                        <span
                            aria-label="live"
                            className="ml-1 inline-block size-1.5 rounded-full bg-success align-middle"
                        />
                    )}
                </td>

                <td
                    className={`${MEDIUM_ONLY} whitespace-nowrap px-2 py-2 text-right tabular-nums text-muted`}
                    title="Average players, over time"
                >
                    {item.server?.avgUsers ?? 0}
                </td>

                <td
                    className={`${MEDIUM_ONLY} max-w-40 truncate px-2 py-2 text-muted`}
                >
                    {result?.map ?? item.server?.map ?? '—'}
                </td>

                <td
                    className={`${WIDE_ONLY} max-w-52 truncate px-2 py-2 font-mono text-xs text-muted`}
                >
                    {/* The address the player would actually connect to. Null
                        when the owner hid it, which is a different fact from
                        "unknown" — hence the dash rather than a blank. */}
                    {item.server?.host
                        ? `${item.server.host}${
                              item.server.port ? `:${item.server.port}` : ''
                          }`
                        : '—'}
                </td>
            </tr>

            {expanded && (
                /*
                 * ONE row, spanning every column. A second `<tr>` per server
                 * with its own cells would have to re-declare the column widths
                 * and would drift out of alignment the moment a column is added.
                 */
                <tr className="border-b border-border bg-surface">
                    <td colSpan={8} className="p-0">
                        {/*
                            `sticky left-0` pins the panel to the visible edge
                            of the horizontally-scrolling table. Without it the
                            panel is as wide as the widest ROW, so the chart and
                            its last statistic sat off-screen to the right —
                            visible only by scrolling away from the row that
                            opened it.
                        */}
                        <div className="sticky left-0 max-w-3xl px-3 pb-3 pt-1">
                            <ExpandedServer
                                item={item}
                                live={live}
                                onOpen={() => navigate(`/view/server/${item.id}`)}
                            />
                        </div>
                    </td>
                </tr>
            )}
        </>
    )
}

function ExpandedServer({
    item,
    live,
    onOpen,
}: {
    item: ContentSummaryT
    live: ReturnType<typeof useLiveServer<HTMLTableRowElement>>
    onOpen: () => void
}) {
    const result = live.result

    /*
     * The chart wants more than one sample, and a row that has just scrolled
     * into view has at most one. `queryNow` on open buys the second immediately
     * rather than making the user wait out a refresh interval they may have set
     * to minutes; the rest arrive on the ordinary cadence.
     */
    useProbeOnOpen(item)

    return (
        <div className="flex flex-col gap-2">
            {/*
                Deliberately short. This chart sits INSIDE a table, where its
                job is the shape of the line — steady, spiky, dropping — not the
                exact millisecond, which the Ping column already gives. At the
                server page's height it dwarfed the rows around it and made two
                open rows more scrolling than the table saved.
            */}
            <LatencyChart series={live.series} height={56} />

            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted">
                {result?.game && <span>Mode: {result.game}</span>}
                {result?.queriedPort != null && (
                    <span>Query port: {result.queriedPort}</span>
                )}
                {result?.protocol && <span>Protocol: {result.protocol}</span>}
                {item.server?.country && <span>{item.server.country}</span>}

                <button
                    type="button"
                    onClick={(e) => {
                        e.stopPropagation()
                        onOpen()
                    }}
                    className="ml-auto text-accent hover:underline"
                >
                    Open server page
                </button>
            </div>

            {result && result.playerList.length > 0 && (
                <div className="max-h-40 overflow-y-auto rounded-lg border border-border">
                    <table className="w-full text-xs">
                        <tbody>
                            {result.playerList.map((player, i) => (
                                <tr
                                    key={`${player.name}-${i}`}
                                    className="border-b border-border/60 last:border-0"
                                >
                                    <td className="truncate px-2 py-1">
                                        {player.name || '(unnamed)'}
                                    </td>
                                    <td className="w-16 px-2 py-1 text-right tabular-nums text-muted">
                                        {player.score ?? ''}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </div>
    )
}

/**
 * Fire one immediate probe when a row is opened.
 *
 * `queryNow` is taken from a ref rather than a dependency: it is stable by
 * design (see the note on `version` in `use-live-query`), but an effect that
 * probes a server and lists its own probe function as a dependency is one
 * refactor away from an unbounded query loop against someone's game server.
 * The ref makes that impossible rather than merely currently-untrue.
 */
function useProbeOnOpen(item: ContentSummaryT) {
    const { queryNow } = useLiveQuery()
    const fn = useRef(queryNow)

    useEffect(() => {
        fn.current = queryNow
    }, [queryNow])

    useEffect(() => {
        const request = requestFor(item, { wantPlayers: true })

        if (request) void fn.current(request)
        // Keyed on the row's identity only: re-running because `item` was
        // rebuilt by a re-render would probe on every paint.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [item.kind, item.id])
}
