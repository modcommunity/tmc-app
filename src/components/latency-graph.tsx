import { useId, useMemo } from 'react'

import type { LatencySeriesT } from '~/lib/ipc/schemas'

/**
 * Latency over time, drawn as inline SVG.
 *
 * No chart library. These render one per card in a scrolling grid of fifty, so
 * the cost of a general-purpose chart — its layout pass, its scales, its
 * tooltips — is paid fifty times for a line with sixty points and no axes. A
 * path string built from the same numbers is a few dozen lines and stays fast
 * on a phone.
 *
 * The visual language is shared with the pill: green under 60ms, amber under
 * 150, red beyond. Colour is never the only signal — the number is always
 * present next to it — because roughly one man in twelve cannot separate the
 * first two.
 */

/**
 * The five rungs of the ladder, in one place.
 *
 * `rtt == null` means the probe did not come back — a different fact from "slow",
 * and the only rung that is not a number. It is drawn as `TO` rather than a
 * dash so an outage cannot be misread as "not measured yet", which is what a
 * dash means everywhere else in the browser.
 */
export const LATENCY_TIERS = [
    { limit: 90, text: 'text-lat-good', color: 'var(--lat-good)' },
    { limit: 150, text: 'text-lat-fair', color: 'var(--lat-fair)' },
    { limit: 200, text: 'text-lat-poor', color: 'var(--lat-poor)' },
    { limit: Infinity, text: 'text-lat-bad', color: 'var(--lat-bad)' },
] as const

export function latencyTone(rtt: number | null | undefined): {
    text: string
    stroke: string
    fill: string
} {
    if (rtt == null) {
        return {
            text: 'text-lat-dead',
            stroke: 'var(--lat-dead)',
            fill: 'var(--lat-dead)',
        }
    }

    const tier =
        LATENCY_TIERS.find((t) => rtt < t.limit) ??
        LATENCY_TIERS[LATENCY_TIERS.length - 1]!

    return { text: tier.text, stroke: tier.color, fill: tier.color }
}

/**
 * The number itself, coloured by its rung.
 *
 * One component for the card, the table and the server panel, so the three can
 * never disagree about what 149ms looks like. `state` separates the three
 * things a row can be in, which the number alone cannot express:
 *
 *   * `measured` — a round trip came back. Print it.
 *   * `timeout`  — a probe went out and nothing came back. Print `TO`, in red.
 *   * `waiting`  — no probe has resolved yet. Print an ellipsis.
 *   * `none`     — there is nothing to probe (the owner hid the address).
 *                  Print a dash, and say why on hover.
 */
export function LatencyValue({
    rttMs,
    state,
    title,
    className = '',
}: {
    rttMs: number | null | undefined
    state: 'measured' | 'timeout' | 'waiting' | 'none'
    title?: string
    className?: string
}) {
    if (state === 'timeout')
        return (
            <span
                className={`font-semibold tabular-nums text-lat-dead ${className}`}
                title={title ?? 'The server did not answer.'}
            >
                TO
            </span>
        )

    if (state !== 'measured' || rttMs == null)
        return (
            <span
                className={`tabular-nums text-muted ${className}`}
                title={
                    title ??
                    (state === 'waiting'
                        ? 'Measuring…'
                        : 'This server’s owner has hidden its address.')
                }
            >
                {state === 'waiting' ? '…' : '—'}
            </span>
        )

    return (
        <span
            className={`font-semibold tabular-nums ${latencyTone(rttMs).text} ${className}`}
            title={title ?? 'Measured from this device just now.'}
        >
            {rttMs}ms
        </span>
    )
}

type Point = { x: number; y: number; rtt: number | null }

/**
 * Lay the series out in a 0–100 × 0–100 viewBox.
 *
 * `preserveAspectRatio="none"` then stretches it to whatever box the caller
 * gives, which is what lets one component serve a 60×16 sparkline and a
 * full-width chart without recomputing anything.
 *
 * The Y scale is padded by 15% above the observed max and floored at a 40ms
 * span. Without the floor, a rock-steady server whose latency varies by 2ms
 * renders as a dramatic mountain range — technically accurate and completely
 * misleading.
 */
function layout(samples: LatencySeriesT['samples']): {
    points: Point[]
    top: number
} {
    const successful = samples
        .map((s) => s.rttMs)
        .filter((v): v is number => v != null)

    const max = successful.length > 0 ? Math.max(...successful) : 0
    const top = Math.max(40, Math.ceil((max * 1.15) / 10) * 10)

    const span = Math.max(1, samples.length - 1)

    const points = samples.map((sample, index) => {
        const rtt = sample.rttMs

        return {
            x: (index / span) * 100,
            // Inverted: SVG's origin is top-left, and a lower ping should sit
            // higher on the chart.
            y: rtt == null ? 100 : 100 - Math.min(100, (rtt / top) * 100),
            rtt,
        }
    })

    return { points, top }
}

/**
 * Split the series into runs of consecutive successful samples.
 *
 * A dropped probe must break the line rather than being interpolated across.
 * Drawing straight through a timeout would show an outage as a smooth dip,
 * which is the opposite of what the graph is for.
 */
function runs(points: Point[]): Point[][] {
    const out: Point[][] = []
    let current: Point[] = []

    for (const point of points) {
        if (point.rtt == null) {
            if (current.length > 0) out.push(current)
            current = []
            continue
        }

        current.push(point)
    }

    if (current.length > 0) out.push(current)

    return out
}

function toPath(run: Point[]): string {
    if (run.length === 1) {
        // A single point has no line. Draw a hairline so it is visible at all.
        const only = run[0]!

        return `M ${only.x - 0.5} ${only.y} L ${only.x + 0.5} ${only.y}`
    }

    return run
        .map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x.toFixed(2)} ${p.y.toFixed(2)}`)
        .join(' ')
}

/** The compact form, for a browser card. */
export function LatencySparkline({
    series,
    className = 'h-4 w-12',
}: {
    series: LatencySeriesT | undefined
    /**
     * The box to stretch into. It REPLACES the default rather than adding to
     * it: `preserveAspectRatio="none"` means the caller owns both dimensions,
     * and appending `w-full` to a baked-in `w-12` would leave which one wins to
     * stylesheet order rather than to the caller.
     */
    className?: string
}) {
    const { points } = useMemo(
        () => layout(series?.samples ?? []),
        [series?.samples]
    )

    // One point is not a trend. Below two samples the pill's number says
    // everything the sparkline could.
    if (points.length < 2) return null

    const tone = latencyTone(series?.last)

    return (
        <svg
            viewBox="0 0 100 100"
            preserveAspectRatio="none"
            aria-hidden="true"
            className={`shrink-0 ${className}`}
        >
            {runs(points).map((run, i) => (
                <path
                    key={i}
                    d={toPath(run)}
                    fill="none"
                    stroke={tone.stroke}
                    strokeWidth={6}
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    vectorEffect="non-scaling-stroke"
                />
            ))}
        </svg>
    )
}

/** The full chart, for a server's page. */
export function LatencyChart({
    series,
    height = 120,
}: {
    series: LatencySeriesT | undefined
    height?: number
}) {
    const gradientId = useId()

    const { points, top } = useMemo(
        () => layout(series?.samples ?? []),
        [series?.samples]
    )

    if (!series || points.length < 2)
        return (
            <div
                className="flex items-center justify-center rounded-lg border border-border bg-surface text-xs text-muted"
                style={{ height }}
            >
                Measuring…
            </div>
        )

    const tone = latencyTone(series.last)
    const segments = runs(points)

    // The filled area under the line, for the longest run only. Filling every
    // run separately makes an intermittent server look striped.
    const longest = segments.reduce<Point[]>(
        (best, run) => (run.length > best.length ? run : best),
        []
    )

    const area =
        longest.length > 1
            ? `${toPath(longest)} L ${longest[longest.length - 1]!.x.toFixed(2)} 100 L ${longest[0]!.x.toFixed(2)} 100 Z`
            : ''

    return (
        <figure className="flex flex-col gap-2">
            <div
                className="relative overflow-hidden rounded-lg border border-border bg-surface"
                style={{ height }}
            >
                <svg
                    viewBox="0 0 100 100"
                    preserveAspectRatio="none"
                    className="size-full"
                    role="img"
                    aria-label={`Latency over the last ${series.samples.length} checks. Currently ${
                        series.last == null
                            ? 'no response'
                            : `${series.last} milliseconds`
                    }.`}
                >
                    <defs>
                        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                            <stop
                                offset="0%"
                                stopColor={tone.fill}
                                stopOpacity="0.25"
                            />
                            <stop
                                offset="100%"
                                stopColor={tone.fill}
                                stopOpacity="0"
                            />
                        </linearGradient>
                    </defs>

                    {/* Gridlines at a quarter, half and three quarters of the
                        scale. Non-scaling stroke keeps them hairlines however
                        the box is stretched. */}
                    {[25, 50, 75].map((y) => (
                        <line
                            key={y}
                            x1="0"
                            y1={y}
                            x2="100"
                            y2={y}
                            stroke="var(--border)"
                            strokeWidth={1}
                            vectorEffect="non-scaling-stroke"
                        />
                    ))}

                    {area && <path d={area} fill={`url(#${gradientId})`} />}

                    {segments.map((run, i) => (
                        <path
                            key={i}
                            d={toPath(run)}
                            fill="none"
                            stroke={tone.stroke}
                            strokeWidth={2}
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            vectorEffect="non-scaling-stroke"
                        />
                    ))}

                    {/* Failed probes as marks on the floor, so an outage is
                        visible rather than merely a gap. */}
                    {points
                        .filter((p) => p.rtt == null)
                        .map((p, i) => (
                            <line
                                key={`miss-${i}`}
                                x1={p.x}
                                y1="88"
                                x2={p.x}
                                y2="100"
                                stroke="var(--danger)"
                                strokeWidth={2}
                                vectorEffect="non-scaling-stroke"
                                opacity={0.6}
                            />
                        ))}
                </svg>

                <span className="absolute left-2 top-1 text-[0.6rem] text-muted">
                    {top} ms
                </span>
            </div>

            <figcaption className="grid grid-cols-4 gap-2 text-center">
                <Stat label="Now" value={series.last} tone={tone.text} />
                <Stat label="Best" value={series.min} />
                <Stat label="Average" value={series.avg} />
                <Stat label="Jitter" value={series.jitter} />
            </figcaption>

            {series.reliability < 100 && (
                <p className="text-center text-xs text-muted">
                    Answered {series.reliability}% of {series.samples.length} checks
                </p>
            )}

            {/*
                The cache notice.

                Two clusters in one series — a tight fast one and a second at
                the real round trip — is what a cache in front of the query port
                produces: fast on every hit, full price on every miss. The SLOW
                number is the one a player's connection will experience, so it
                is the one the sentence ends on; the fast one is what the server
                claims.

                Worded as an observation rather than an accusation. A CDN, a
                proxy and a game that answers from memory look identical here.
            */}
            {series.cache && (
                <p className="text-center text-xs text-warning">
                    Replies look cached: {series.cache.fastMs}ms on{' '}
                    {Math.round(series.cache.fastShare * 100)}% of checks and{' '}
                    {series.cache.slowMs}ms on the rest — expect closer to{' '}
                    {series.cache.slowMs}ms in game.
                </p>
            )}
        </figure>
    )
}

function Stat({
    label,
    value,
    tone,
}: {
    label: string
    value: number | null
    tone?: string
}) {
    return (
        <div className="flex flex-col">
            <span className={`text-sm font-semibold tabular-nums ${tone ?? ''}`}>
                {value == null ? '—' : `${value}ms`}
            </span>
            <span className="text-[0.65rem] uppercase tracking-wide text-muted">
                {label}
            </span>
        </div>
    )
}
