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

export function latencyTone(rtt: number | null | undefined): {
    text: string
    stroke: string
    fill: string
} {
    if (rtt == null) {
        return {
            text: 'text-muted',
            stroke: 'var(--muted)',
            fill: 'var(--muted)',
        }
    }

    if (rtt < 60) {
        return {
            text: 'text-success',
            stroke: 'var(--success)',
            fill: 'var(--success)',
        }
    }

    if (rtt < 150) {
        return {
            text: 'text-warning',
            stroke: 'var(--warning)',
            fill: 'var(--warning)',
        }
    }

    return {
        text: 'text-danger',
        stroke: 'var(--danger)',
        fill: 'var(--danger)',
    }
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
    className = '',
}: {
    series: LatencySeriesT | undefined
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
            className={`h-4 w-12 shrink-0 ${className}`}
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
                        series.last == null ? 'no response' : `${series.last} milliseconds`
                    }.`}
                >
                    <defs>
                        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                            <stop offset="0%" stopColor={tone.fill} stopOpacity="0.25" />
                            <stop offset="100%" stopColor={tone.fill} stopOpacity="0" />
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
