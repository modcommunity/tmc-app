import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useParams, useSearchParams } from 'react-router-dom'
import {
    FiActivity,
    FiCheck,
    FiCopy,
    FiExternalLink,
    FiLayout,
    FiPause,
    FiPlay,
    FiSidebar,
    FiTerminal,
    FiUsers,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import type { ContentSummaryT } from '~/lib/api/contract'
import { ipc, type QueryRequestT } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import type { RconServerT, ServerQueryResultT } from '~/lib/ipc/schemas'
import { useSettings } from '~/lib/settings/provider'
import { requestFor } from '~/lib/hooks/use-live-query'
import {
    HISTORY_CAPACITY,
    LAYOUT_KEY,
    RANGES,
    RingBuffer,
    computeStats,
    formatSpan,
    inRange,
    mergeHistory,
    nearestIndex,
    parseLayoutMode,
    parseRange,
    type LayoutMode,
    type MonitorSample,
    type RangeId,
} from '~/lib/monitor'
import { GameIcon } from '~/components/game-icon'
import { LatencyValue, latencyTone } from '~/components/latency-graph'
import TimeChart from '~/components/monitor-chart'
import { Console, ServerForm } from '~/routes/rcon'

const RANGE_KEY = 'tmc.monitor.range'
const DETAILS_KEY = 'tmc.monitor.details'
const DEFAULT_INTERVAL_MS = 1000
const MIN_INTERVAL_MS = 500
/** The roster and rules are heavier than a ping, so fetch them every Nth probe. */
const FULL_EVERY = 5

type Ident = {
    host: string
    port: number
    appId: number | null
    name: string
    serverId: number | null
}

const clock = (at: number) =>
    new Date(at).toLocaleTimeString(undefined, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
    })

function read(key: string): string | null {
    try {
        return window.localStorage.getItem(key)
    } catch {
        return null
    }
}

function write(key: string, value: string) {
    try {
        window.localStorage.setItem(key, value)
    } catch {
        // Private mode or a full quota: the choice just does not persist.
    }
}

function flag(code: string | null | undefined): string {
    if (!code || !/^[a-z]{2}$/i.test(code)) return ''

    return String.fromCodePoint(
        ...[...code.toUpperCase()].map((c) => 0x1f1e6 + c.charCodeAt(0) - 65)
    )
}

// -------------------------------------------------------------------- Feed

/**
 * Probe a server on a loop and keep a rolling history of the answers.
 *
 * The window owns its history rather than reading the browser's live registry:
 * it is a separate document with its own memory, and it must keep sampling
 * while the main window is on another screen. The registry's series is only
 * used once, to seed the left edge so a fresh window is not empty.
 */
function useFeed(base: QueryRequestT | null, paused: boolean, intervalMs: number) {
    const [live, setLive] = useState<MonitorSample[]>([])
    const [seed, setSeed] = useState<MonitorSample[]>([])
    const [result, setResult] = useState<ServerQueryResultT | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [lastAt, setLastAt] = useState<number | null>(null)

    const ringRef = useRef<RingBuffer<MonitorSample> | null>(null)

    const key = base ? `${base.host.trim().toLowerCase()}:${base.port}` : null

    useEffect(() => {
        if (!key) return

        let cancelled = false

        ipc.latencySeries([key])
            .then((series) => {
                if (cancelled) return

                setSeed(
                    (series[0]?.samples ?? []).map((s) => ({
                        at: s.at,
                        rtt: s.rttMs,
                        players: null,
                        maxPlayers: null,
                    }))
                )
            })
            .catch(() => undefined)

        return () => {
            cancelled = true
        }
    }, [key])

    useEffect(() => {
        if (!base || paused) return

        // Lives across pauses so the series continues instead of restarting.
        const ring = (ringRef.current ??= new RingBuffer<MonitorSample>(HISTORY_CAPACITY))
        let timer: ReturnType<typeof setTimeout> | undefined
        let stopped = false
        let n = 0
        let haveFull = false

        const tick = async () => {
            const started = Date.now()
            const wantPlayers = !haveFull || n % FULL_EVERY === 0

            try {
                const out = await ipc.queryServer({ ...base, wantPlayers })
                const r = out.result
                const ok = Boolean(r && r.online && !out.error)

                if (stopped) return

                ring.push({
                    at: Date.now(),
                    rtt: ok && r ? r.rttMs : null,
                    players: r?.players ?? null,
                    maxPlayers: r?.maxPlayers ?? null,
                })

                if (r && r.online) {
                    if (wantPlayers) haveFull = true

                    setResult((prev) =>
                        wantPlayers || !prev
                            ? r
                            : { ...r, playerList: prev.playerList, rules: prev.rules }
                    )
                }

                setError(out.error ?? (r && !r.online ? 'Offline' : null))
            } catch (err) {
                if (stopped) return

                ring.push({ at: Date.now(), rtt: null, players: null, maxPlayers: null })
                setError(messageOf(err))
            }

            n += 1
            setLive(ring.toArray())
            setLastAt(Date.now())

            if (!stopped)
                timer = setTimeout(
                    () => void tick(),
                    Math.max(0, intervalMs - (Date.now() - started))
                )
        }

        void tick()

        return () => {
            stopped = true
            if (timer) clearTimeout(timer)
        }
    }, [base, paused, intervalMs])

    const samples = useMemo(() => mergeHistory(seed, live), [seed, live])

    return { samples, result, error, lastAt }
}

// ------------------------------------------------------------------- Route

export default function MonitorRoute() {
    const params = useParams<{ id?: string }>()
    const [search] = useSearchParams()
    const { app } = useSettings()

    const serverId = params.id ?? null

    const detail = useQuery({
        queryKey: ['content', 'server', serverId],
        queryFn: () => api.content('server', serverId!),
        enabled: serverId !== null,
        staleTime: 60 * 1000,
    })

    const summary: ContentSummaryT | null = detail.data?.summary ?? null

    const ident: Ident | null = useMemo(() => {
        if (serverId !== null) {
            if (!summary?.server?.host) return null

            return {
                host: summary.server.host,
                port: summary.server.port ?? 0,
                appId: summary.app?.id ?? null,
                name: summary.name,
                serverId: Number.isFinite(Number(serverId)) ? Number(serverId) : null,
            }
        }

        const host = search.get('host')
        const port = Number(search.get('port'))

        if (!host || !Number.isInteger(port) || port <= 0) return null

        const appRaw = search.get('app')

        return {
            host,
            port,
            appId: appRaw && Number.isFinite(Number(appRaw)) ? Number(appRaw) : null,
            name: search.get('name') || `${host}:${port}`,
            serverId: null,
        }
    }, [serverId, summary, search])

    const base: QueryRequestT | null = useMemo(() => {
        if (serverId !== null) {
            if (!summary) return null

            return requestFor(summary)
        }

        if (!ident) return null

        // No catalogue entry to say how to query it: Rust falls back to a plain
        // TCP probe, or a plugin for the game when `appId` selects one.
        return {
            id: `addr:${ident.host}:${ident.port}`,
            appId: ident.appId,
            host: ident.host,
            port: ident.port,
            wantPlayers: false,
        }
    }, [serverId, summary, ident])

    const [paused, setPaused] = useState(false)
    const [range, setRange] = useState<RangeId>(() => parseRange(read(RANGE_KEY)))
    const [layout, setLayout] = useState<LayoutMode>(() =>
        parseLayoutMode(read(LAYOUT_KEY))
    )
    const [details, setDetails] = useState(() => read(DETAILS_KEY) !== 'off')

    const intervalMs = Math.max(
        MIN_INTERVAL_MS,
        app?.latencyIntervalMs ?? DEFAULT_INTERVAL_MS
    )

    const feed = useFeed(base, paused, intervalMs)

    const name = feed.result?.name || ident?.name || 'Server monitor'

    useEffect(() => {
        document.title = `${name} — Monitor`

        void (async () => {
            if (!('__TAURI_INTERNALS__' in window)) return

            try {
                const { getCurrentWindow } = await import('@tauri-apps/api/window')

                await getCurrentWindow().setTitle(`${name} — Monitor`)
            } catch {
                // The document title above is enough when the call is refused.
            }
        })()
    }, [name])

    const chooseLayout = (next: LayoutMode) => {
        setLayout(next)
        write(LAYOUT_KEY, next)
    }

    const chooseRange = (next: RangeId) => {
        setRange(next)
        write(RANGE_KEY, next)
    }

    if (serverId !== null && detail.isPending)
        return <Centered>Loading server…</Centered>

    if (serverId !== null && detail.isError)
        return <Centered>Could not load this server: {messageOf(detail.error)}</Centered>

    if (!base || !ident)
        return (
            <Centered>
                This server has no address the app is allowed to probe. Its owner may have
                hidden its network details.
            </Centered>
        )

    const showGraph = layout !== 'rcon'
    const showRcon = layout !== 'graph'

    return (
        <div className="flex h-screen min-h-0 flex-col bg-background text-foreground">
            <Header
                ident={ident}
                summary={summary}
                name={name}
                result={feed.result}
                samples={feed.samples}
                error={feed.error}
                paused={paused}
            />

            <Toolbar
                range={range}
                onRange={chooseRange}
                paused={paused}
                onPause={() => setPaused((p) => !p)}
                layout={layout}
                onLayout={chooseLayout}
                details={details}
                onDetails={() => {
                    setDetails((d) => {
                        write(DETAILS_KEY, d ? 'off' : 'on')

                        return !d
                    })
                }}
                showDetails={showGraph}
                intervalMs={intervalMs}
                lastAt={feed.lastAt}
            />

            <main
                className={`flex min-h-0 flex-1 gap-3 p-3 ${
                    layout === 'split'
                        ? 'flex-col min-[1100px]:flex-row'
                        : 'flex-col'
                }`}
            >
                {showGraph && (
                    <GraphArea
                        samples={feed.samples}
                        range={range}
                        paused={paused}
                        result={feed.result}
                        summary={summary}
                        details={details}
                        compact={layout === 'split'}
                    />
                )}

                {showRcon && (
                    <section
                        aria-label="RCON"
                        className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-xl border border-border bg-surface"
                    >
                        <RconPanel ident={ident} />
                    </section>
                )}
            </main>
        </div>
    )
}

function Centered({ children }: { children: React.ReactNode }) {
    return (
        <div className="flex h-screen items-center justify-center p-6 text-center text-sm text-muted">
            <p className="max-w-md">{children}</p>
        </div>
    )
}

// ------------------------------------------------------------------ Header

function Header({
    ident,
    summary,
    name,
    result,
    samples,
    error,
    paused,
}: {
    ident: Ident
    summary: ContentSummaryT | null
    name: string
    result: ServerQueryResultT | null
    samples: MonitorSample[]
    error: string | null
    paused: boolean
}) {
    const [copied, setCopied] = useState(false)
    const address = `${ident.host}:${ident.port}`
    const last = samples[samples.length - 1] ?? null
    const online = last ? last.rtt !== null : null
    const players = result?.players ?? summary?.server?.curUsers ?? null
    const max = result?.maxPlayers ?? summary?.server?.maxUsers ?? null
    const map = result?.map ?? summary?.server?.map ?? null
    const country = summary?.server?.country ?? null
    const code = summary?.server?.countryCode ?? null

    const copy = async () => {
        try {
            await navigator.clipboard.writeText(address)
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1500)
        } catch {
            // Clipboard refused; the address is selectable text as a fallback.
        }
    }

    const dot =
        online === null
            ? 'bg-muted'
            : online
              ? 'bg-success motion-safe:animate-pulse'
              : 'bg-danger'

    return (
        <header className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-border bg-app-chrome px-4 py-3">
            <GameIcon app={summary?.app ?? null} size="lg" className="rounded-lg" />

            <div className="min-w-0 flex-1 basis-60">
                <div className="flex items-center gap-2">
                    <span
                        role="img"
                        aria-label={online === null ? 'Waiting' : online ? 'Online' : 'Offline'}
                        className={`size-2.5 shrink-0 rounded-full ${dot}`}
                    />
                    <h1 className="selectable truncate text-base font-bold" title={name}>
                        {name}
                    </h1>
                </div>
                <p className="selectable mt-0.5 flex flex-wrap items-center gap-x-2 text-xs text-muted">
                    <span className="font-mono">{address}</span>
                    <button
                        type="button"
                        onClick={() => void copy()}
                        title="Copy address"
                        aria-label="Copy address"
                        className="inline-flex items-center gap-1 rounded px-1 hover:text-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                    >
                        {copied ? <FiCheck className="size-3" /> : <FiCopy className="size-3" />}
                        {copied ? 'Copied' : 'Copy'}
                    </button>
                    {summary?.app && <span>· {summary.app.name}</span>}
                    {(country || code) && (
                        <span>
                            · {flag(code)} {country ?? code}
                        </span>
                    )}
                </p>
            </div>

            <dl className="flex flex-wrap items-center gap-x-5 gap-y-1 text-xs">
                <Fact label="Map" value={map ?? '—'} />
                <Fact
                    label="Players"
                    value={
                        players === null
                            ? '—'
                            : `${players}${max !== null ? ` / ${max}` : ''}`
                    }
                />
                <div className="flex flex-col">
                    <dt className="text-[10px] uppercase tracking-wide text-muted">Ping</dt>
                    <dd className="text-sm font-semibold">
                        <LatencyValue rttMs={last?.rtt ?? null} state={last ? (last.rtt === null ? 'timeout' : 'measured') : 'waiting'} />
                    </dd>
                </div>
            </dl>

            <div className="flex items-center gap-2">
                {summary && (
                    <button
                        type="button"
                        onClick={() => void openServerPage(summary)}
                        title="Open the server page in the main window"
                        className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface px-3 py-1.5 text-xs font-medium transition-colors hover:border-accent hover:text-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                    >
                        <FiExternalLink className="size-3.5" /> Server page
                    </button>
                )}
                {summary?.server?.connectUrl && (
                    <button
                        type="button"
                        onClick={() => void openServerPage(summary)}
                        title="Opens the server page in the main window, where Join runs the launch checks"
                        className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                    >
                        <FiPlay className="size-3.5" /> Join
                    </button>
                )}
            </div>

            {(error || paused) && (
                <p
                    role="status"
                    className="basis-full text-xs text-warning"
                >
                    {paused ? 'Paused. ' : ''}
                    {error ? `Last probe: ${error}` : ''}
                </p>
            )}
        </header>
    )
}

function Fact({ label, value }: { label: string; value: string }) {
    return (
        <div className="flex min-w-0 flex-col">
            <dt className="text-[10px] uppercase tracking-wide text-muted">{label}</dt>
            <dd className="selectable max-w-40 truncate text-sm font-semibold" title={value}>
                {value}
            </dd>
        </div>
    )
}

/**
 * The main window owns navigation and the play dialog, so the monitor hands
 * both off to it rather than duplicating them. A hash link opens the page in
 * this window's own router instead when there is no main window to ask.
 */
async function openServerPage(summary: ContentSummaryT) {
    const hash = `#/view/server/${summary.id}`

    if ('__TAURI_INTERNALS__' in window) {
        try {
            const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow')
            const main = await WebviewWindow.getByLabel('main')

            if (main) {
                await main.emit('monitor:navigate', { path: hash.slice(1) })
                await main.unminimize()
                await main.setFocus()

                return
            }
        } catch {
            // Fall through to navigating here.
        }
    }

    window.location.hash = hash
}

// ----------------------------------------------------------------- Toolbar

function Toolbar({
    range,
    onRange,
    paused,
    onPause,
    layout,
    onLayout,
    details,
    onDetails,
    showDetails,
    intervalMs,
    lastAt,
}: {
    range: RangeId
    onRange: (r: RangeId) => void
    paused: boolean
    onPause: () => void
    layout: LayoutMode
    onLayout: (l: LayoutMode) => void
    details: boolean
    onDetails: () => void
    showDetails: boolean
    intervalMs: number
    lastAt: number | null
}) {
    const seg =
        'inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium transition-colors focus-visible:z-10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent'

    const modes: { id: LayoutMode; label: string; icon: React.ReactNode }[] = [
        { id: 'graph', label: 'Graph', icon: <FiActivity className="size-3.5" /> },
        { id: 'split', label: 'Split', icon: <FiLayout className="size-3.5" /> },
        { id: 'rcon', label: 'RCON', icon: <FiTerminal className="size-3.5" /> },
    ]

    return (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-border px-4 py-2">
            {layout !== 'rcon' && (
                <div role="group" aria-label="Time range" className="inline-flex overflow-hidden rounded-lg border border-border">
                    {RANGES.map((r) => (
                        <button
                            key={r.id}
                            type="button"
                            aria-pressed={range === r.id}
                            onClick={() => onRange(r.id)}
                            className={`${seg} ${
                                range === r.id
                                    ? 'bg-accent text-white'
                                    : 'bg-surface hover:text-accent'
                            }`}
                        >
                            {r.label}
                        </button>
                    ))}
                </div>
            )}

            <button
                type="button"
                aria-pressed={paused}
                onClick={onPause}
                className={`${seg} rounded-lg border border-border ${
                    paused ? 'bg-warning/15 text-warning' : 'bg-surface hover:text-accent'
                }`}
            >
                {paused ? <FiPlay className="size-3.5" /> : <FiPause className="size-3.5" />}
                {paused ? 'Resume' : 'Pause'}
            </button>

            <span className="hidden text-[11px] text-muted min-[560px]:inline">
                every {(intervalMs / 1000).toFixed(intervalMs % 1000 === 0 ? 0 : 1)}s
                {lastAt ? ` · updated ${clock(lastAt)}` : ''}
            </span>

            <div className="ml-auto flex items-center gap-2">
                {showDetails && (
                    <button
                        type="button"
                        aria-pressed={details}
                        onClick={onDetails}
                        title="Show or hide the players and details panel"
                        className={`${seg} rounded-lg border border-border ${
                            details ? 'bg-surface-2 text-accent' : 'bg-surface hover:text-accent'
                        }`}
                    >
                        <FiSidebar className="size-3.5" /> Details
                    </button>
                )}

                <div role="group" aria-label="Layout" className="inline-flex overflow-hidden rounded-lg border border-border">
                    {modes.map((m) => (
                        <button
                            key={m.id}
                            type="button"
                            aria-pressed={layout === m.id}
                            onClick={() => onLayout(m.id)}
                            className={`${seg} ${
                                layout === m.id
                                    ? 'bg-accent text-white'
                                    : 'bg-surface hover:text-accent'
                            }`}
                        >
                            {m.icon}
                            <span className="hidden min-[520px]:inline">{m.label}</span>
                        </button>
                    ))}
                </div>
            </div>
        </div>
    )
}

// ------------------------------------------------------------------- Graph

function GraphArea({
    samples,
    range,
    paused,
    result,
    summary,
    details,
    compact,
}: {
    samples: MonitorSample[]
    range: RangeId
    paused: boolean
    result: ServerQueryResultT | null
    summary: ContentSummaryT | null
    details: boolean
    compact: boolean
}) {
    const [hoverAt, setHoverAt] = useState<number | null>(null)

    // The right edge is the newest probe, so a paused window holds still
    // instead of sliding as the clock moves.
    const newest = samples[samples.length - 1]?.at ?? 0
    const t1 = newest || 1
    const visible = useMemo(() => inRange(samples, range, t1), [samples, range, t1])
    const rangeMs = RANGES.find((r) => r.id === range)?.ms ?? null
    const first = visible[0]?.at ?? t1
    const t0 = rangeMs === null ? Math.min(first, t1 - 10_000) : t1 - rangeMs

    const stats = useMemo(() => computeStats(visible), [visible])

    const onHover = useCallback(
        (at: number | null) => {
            if (at === null || !visible.length) {
                setHoverAt(null)

                return
            }

            setHoverAt(visible[nearestIndex(visible.map((s) => s.at), at)]!.at)
        },
        [visible]
    )

    const tone = latencyTone(stats.avg)
    const hasPlayers = visible.some((s) => s.players !== null)

    return (
        <div
            className={`flex min-h-0 min-w-0 flex-1 gap-3 ${
                compact ? 'flex-col' : 'flex-col min-[860px]:flex-row'
            }`}
        >
            <div className="flex min-h-0 min-w-0 flex-[3] flex-col gap-3">
                <StatCards stats={stats} spanMs={visible.length ? t1 - first : 0} />

                <section
                    aria-label="Latency"
                    className="flex min-h-[140px] flex-[3] flex-col rounded-xl border border-border bg-surface p-3"
                >
                    <h2 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted">
                        Latency (ms)
                    </h2>
                    <div className="min-h-0 flex-1">
                        <TimeChart
                            samples={visible}
                            value={(s) => s.rtt}
                            t0={t0}
                            t1={t1}
                            color={tone.stroke}
                            hoverAt={hoverAt}
                            onHover={onHover}
                            floor={50}
                            unit="ms"
                            label="Latency"
                            timeouts
                            tooltip={(s) => [
                                clock(s.at),
                                s.rtt === null ? 'Timed out' : `Ping ${Math.round(s.rtt)} ms`,
                                s.players === null
                                    ? 'Players —'
                                    : `Players ${s.players}${s.maxPlayers ? ` / ${s.maxPlayers}` : ''}`,
                            ]}
                        />
                    </div>
                </section>

                <section
                    aria-label="Players over time"
                    className="flex min-h-[90px] flex-1 flex-col rounded-xl border border-border bg-surface p-3"
                >
                    <h2 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted">
                        Players
                    </h2>
                    <div className="min-h-0 flex-1">
                        <TimeChart
                            samples={visible}
                            value={(s) => s.players}
                            t0={t0}
                            t1={t1}
                            color="var(--accent)"
                            hoverAt={hoverAt}
                            onHover={onHover}
                            floor={4}
                            unit=""
                            label="Players"
                            integer
                            step
                            tooltip={(s) => [
                                clock(s.at),
                                s.players === null
                                    ? 'Players —'
                                    : `Players ${s.players}${s.maxPlayers ? ` / ${s.maxPlayers}` : ''}`,
                            ]}
                        />
                    </div>
                    {!hasPlayers && visible.length > 0 && (
                        <p className="text-[11px] text-muted">
                            This server does not report a player count.
                        </p>
                    )}
                </section>
            </div>

            {details && (
                <SidePanel
                    result={result}
                    summary={summary}
                    paused={paused}
                    wide={!compact}
                />
            )}
        </div>
    )
}

function StatCards({
    stats,
    spanMs,
}: {
    stats: ReturnType<typeof computeStats>
    spanMs: number
}) {
    const ms = (v: number | null) => (v === null ? '—' : `${Math.round(v)}`)

    const cards: { label: string; value: string; unit: string; tone?: string }[] = [
        { label: 'Min', value: ms(stats.min), unit: 'ms', tone: latencyTone(stats.min).text },
        { label: 'Avg', value: ms(stats.avg), unit: 'ms', tone: latencyTone(stats.avg).text },
        { label: 'Max', value: ms(stats.max), unit: 'ms', tone: latencyTone(stats.max).text },
        { label: 'P95', value: ms(stats.p95), unit: 'ms', tone: latencyTone(stats.p95).text },
        {
            label: 'Jitter',
            value: stats.jitter === null ? '—' : stats.jitter.toFixed(1),
            unit: 'ms',
        },
        {
            label: 'Loss',
            value: stats.lossPct.toFixed(stats.lossPct % 1 === 0 ? 0 : 1),
            unit: '%',
            tone: stats.lossPct === 0 ? 'text-success' : stats.lossPct < 5 ? 'text-warning' : 'text-danger',
        },
    ]

    return (
        <div>
            <ul className="grid grid-cols-3 gap-2 min-[640px]:grid-cols-6">
                {cards.map((c) => (
                    <li
                        key={c.label}
                        className="rounded-xl border border-border bg-surface px-3 py-2"
                    >
                        <p className="text-[10px] font-semibold uppercase tracking-wide text-muted">
                            {c.label}
                        </p>
                        <p className={`text-lg font-bold tabular-nums leading-tight ${c.tone ?? ''}`}>
                            {c.value}
                            <span className="ml-0.5 text-xs font-medium text-muted">{c.value === '—' ? '' : c.unit}</span>
                        </p>
                    </li>
                ))}
            </ul>
            <p className="mt-1 text-[11px] text-muted">
                {stats.count} probes
                {spanMs > 0 ? ` over ${formatSpan(spanMs)}` : ''}
                {stats.lost > 0 ? ` · ${stats.lost} unanswered` : ''}
            </p>
        </div>
    )
}

// -------------------------------------------------------------- Side panel

function SidePanel({
    result,
    summary,
    paused,
    wide,
}: {
    result: ServerQueryResultT | null
    summary: ContentSummaryT | null
    paused: boolean
    wide: boolean
}) {
    const [tab, setTab] = useState<'players' | 'details'>('players')
    const players = result?.playerList ?? []

    const tabBtn = (id: 'players' | 'details', label: string, icon: React.ReactNode) => (
        <button
            type="button"
            role="tab"
            aria-selected={tab === id}
            onClick={() => setTab(id)}
            className={`inline-flex flex-1 items-center justify-center gap-1.5 px-3 py-2 text-xs font-medium transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent ${
                tab === id ? 'border-b-2 border-accent text-accent' : 'text-muted hover:text-foreground'
            }`}
        >
            {icon}
            {label}
        </button>
    )

    return (
        <aside
            aria-label="Players and details"
            className={`flex min-h-[200px] min-w-0 flex-col overflow-hidden rounded-xl border border-border bg-surface ${
                wide ? 'min-[860px]:w-72 min-[860px]:shrink-0' : 'max-h-64'
            } flex-1 min-[860px]:flex-none`}
        >
            <div role="tablist" className="flex border-b border-border">
                {tabBtn('players', `Players${players.length ? ` (${players.length})` : ''}`, <FiUsers className="size-3.5" />)}
                {tabBtn('details', 'Details', <FiSidebar className="size-3.5" />)}
            </div>

            <div className="min-h-0 flex-1 overflow-auto">
                {tab === 'players' ? (
                    <PlayerList players={players} result={result} paused={paused} />
                ) : (
                    <Details result={result} summary={summary} />
                )}
            </div>
        </aside>
    )
}

function duration(seconds: number | null): string {
    if (seconds === null || !Number.isFinite(seconds)) return '—'

    const s = Math.max(0, Math.round(seconds))
    const h = Math.floor(s / 3600)
    const m = Math.floor((s % 3600) / 60)

    return h > 0 ? `${h}h ${String(m).padStart(2, '0')}m` : `${m}m ${String(s % 60).padStart(2, '0')}s`
}

function PlayerList({
    players,
    result,
    paused,
}: {
    players: ServerQueryResultT['playerList']
    result: ServerQueryResultT | null
    paused: boolean
}) {
    if (!result)
        return <p className="p-4 text-xs text-muted">Waiting for the first answer…</p>

    if (!players.length)
        return (
            <p className="p-4 text-xs text-muted">
                {result.players
                    ? 'This server reports a count but not who is on it.'
                    : 'Nobody is on the server right now.'}
            </p>
        )

    const sorted = [...players].sort((a, b) => (b.score ?? -Infinity) - (a.score ?? -Infinity))

    return (
        <table className="w-full text-xs">
            <thead className="sticky top-0 bg-surface text-left text-[10px] uppercase tracking-wide text-muted">
                <tr>
                    <th className="px-3 py-1.5 font-semibold">Name</th>
                    <th className="px-2 py-1.5 text-right font-semibold">Score</th>
                    <th className="px-3 py-1.5 text-right font-semibold">Time</th>
                </tr>
            </thead>
            <tbody>
                {sorted.map((p, i) => (
                    <tr key={`${p.name}-${i}`} className="border-t border-border/60">
                        <td className="selectable max-w-0 truncate px-3 py-1.5" title={p.name}>
                            {p.name || <span className="text-muted">connecting…</span>}
                        </td>
                        <td className="px-2 py-1.5 text-right tabular-nums">{p.score ?? '—'}</td>
                        <td className="px-3 py-1.5 text-right tabular-nums text-muted">
                            {duration(p.duration)}
                        </td>
                    </tr>
                ))}
                {paused && (
                    <tr>
                        <td colSpan={3} className="px-3 py-2 text-[11px] text-muted">
                            Paused: this list is as of the last probe.
                        </td>
                    </tr>
                )}
            </tbody>
        </table>
    )
}

function Details({
    result,
    summary,
}: {
    result: ServerQueryResultT | null
    summary: ContentSummaryT | null
}) {
    const [filter, setFilter] = useState('')
    const server = summary?.server ?? null
    const rules = result?.rules ?? {}

    const rows: [string, string][] = [
        ['Game', result?.game ?? summary?.app?.name ?? '—'],
        ['Version', result?.version ?? server?.version ?? '—'],
        ['Mode', server?.gameMode ?? '—'],
        ['Region', server?.region ?? '—'],
        ['Protocol', result?.protocol ?? '—'],
        [
            'OS',
            rules.os ?? rules.platform ?? rules.sv_os ?? '—',
        ],
        [
            'Anti-cheat',
            (result?.secure ?? server?.secure) === null ||
            (result?.secure ?? server?.secure) === undefined
                ? '—'
                : (result?.secure ?? server?.secure)
                  ? 'Secured'
                  : 'None',
        ],
        [
            'Password',
            (result?.password ?? server?.password) === null ||
            (result?.password ?? server?.password) === undefined
                ? '—'
                : (result?.password ?? server?.password)
                  ? 'Required'
                  : 'Open',
        ],
        ['Bots', result?.bots != null ? String(result.bots) : '—'],
    ]

    const entries = Object.entries(rules)
        .filter(([k, v]) => {
            const q = filter.trim().toLowerCase()

            return !q || k.toLowerCase().includes(q) || v.toLowerCase().includes(q)
        })
        .sort(([a], [b]) => a.localeCompare(b))

    return (
        <div className="p-3 text-xs">
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5">
                {rows.map(([k, v]) => (
                    <div key={k} className="contents">
                        <dt className="text-muted">{k}</dt>
                        <dd className="selectable min-w-0 truncate text-right font-medium" title={v}>
                            {v}
                        </dd>
                    </div>
                ))}
            </dl>

            {summary && summary.tags.length > 0 && (
                <div className="mt-3 flex flex-wrap gap-1">
                    {summary.tags.map((t) => (
                        <span key={t.id} className="rounded-full border border-border px-2 py-0.5 text-[10px] text-muted">
                            {t.name}
                        </span>
                    ))}
                </div>
            )}

            <h3 className="mb-1.5 mt-4 text-[10px] font-semibold uppercase tracking-wide text-muted">
                Rules{Object.keys(rules).length ? ` (${Object.keys(rules).length})` : ''}
            </h3>

            {Object.keys(rules).length === 0 ? (
                <p className="text-muted">This server does not publish any rules.</p>
            ) : (
                <>
                    <input
                        type="search"
                        value={filter}
                        onChange={(e) => setFilter(e.target.value)}
                        placeholder="Filter rules"
                        aria-label="Filter rules"
                        className="mb-2 w-full rounded-md border border-border bg-background px-2 py-1 text-xs outline-none focus:border-accent"
                    />
                    <dl className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-x-3 gap-y-1">
                        {entries.map(([k, v]) => (
                            <div key={k} className="contents">
                                <dt className="selectable truncate font-mono text-muted" title={k}>
                                    {k}
                                </dt>
                                <dd className="selectable truncate text-right font-mono" title={v}>
                                    {v}
                                </dd>
                            </div>
                        ))}
                    </dl>
                </>
            )}
        </div>
    )
}

// -------------------------------------------------------------------- RCON

/**
 * The RCON console for this server, reusing the RCON screen's own Console.
 *
 * Matching follows the server page's rule: a saved login linked to this TMC
 * server wins, otherwise one on the same host (and game, when both are known).
 * With none saved it offers the same form, prefilled.
 */
function RconPanel({ ident }: { ident: Ident }) {
    const [saved, setSaved] = useState<RconServerT[] | null>(null)
    const [unavailable, setUnavailable] = useState(false)
    const [adding, setAdding] = useState(false)
    const [editing, setEditing] = useState(false)

    const load = useCallback(async () => {
        try {
            setSaved(await ipc.rconList())
            setUnavailable(false)
        } catch {
            setUnavailable(true)
        }
    }, [])

    useEffect(() => {
        queueMicrotask(() => void load())
    }, [load])

    if (unavailable)
        return (
            <p className="p-6 text-center text-sm text-muted">
                RCON is only available in the desktop app.
            </p>
        )

    if (saved === null) return <p className="p-6 text-center text-sm text-muted">Loading…</p>

    const match =
        saved.find((s) => ident.serverId !== null && s.serverId === ident.serverId) ??
        saved.find(
            (s) =>
                s.host.toLowerCase() === ident.host.toLowerCase() &&
                (ident.appId === null || s.appId === null || s.appId === ident.appId)
        )

    const form = (
        <div className="overflow-auto p-3">
            <ServerForm
                server={editing ? match : undefined}
                preset={{
                    name: ident.name,
                    host: ident.host,
                    port: 27015,
                    appId: ident.appId,
                    serverId: ident.serverId,
                }}
                onCancel={() => {
                    setAdding(false)
                    setEditing(false)
                }}
                onSaved={async () => {
                    setAdding(false)
                    setEditing(false)
                    await load()
                }}
            />
        </div>
    )

    if (adding || (editing && match)) return form

    if (match)
        return (
            <div className="flex min-h-0 flex-1 flex-col overflow-auto p-3">
                <Console
                    key={match.id}
                    server={match}
                    onEdit={() => setEditing(true)}
                    onChanged={load}
                    onDeleted={load}
                />
            </div>
        )

    return (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
            <FiTerminal className="size-6 text-muted" />
            <p className="max-w-sm text-sm text-muted">
                Administer this server? Save its RCON login to run commands from this window.
                It stays encrypted on this device.
            </p>
            <button
                type="button"
                onClick={() => setAdding(true)}
                className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white transition-opacity hover:opacity-90 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
            >
                Set up RCON
            </button>
        </div>
    )
}
