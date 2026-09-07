import { useMemo } from 'react'

import { formatSpeed } from '~/lib/downloads/provider'

/**
 * A download's recent speed, as an SVG area.
 *
 * Hand-rolled, for the same reason `latency-graph` is: this renders once per
 * row in a list that can hold hundreds, and a general-purpose chart library's
 * layout pass and scale objects get paid every one of those times for a
 * sixty-point line with no axes on it.
 *
 * THE Y SCALE IS THE INTERESTING PART
 * -----------------------------------
 * It is the window's own maximum, with a floor. Scaling to a fixed ceiling
 * would draw a flat line along the bottom for anybody on a slow connection;
 * scaling to the maximum alone turns a steady 4 MB/s into a jagged mountain
 * range, because the tiny variation between samples fills the whole height.
 * The floor is what stops that — a rock-steady download looks steady.
 *
 * A DROPPED SAMPLE IS A ZERO, NOT A GAP
 * -------------------------------------
 * The opposite of the latency graph's rule, and deliberately: a latency probe
 * that fails means "we do not know", while a download tick with no bytes in it
 * means the transfer really did stall for half a second. Interpolating over
 * that would hide exactly the thing somebody watching this graph is looking
 * for.
 */

type Props = {
    /** Bytes per second, oldest first. */
    samples: number[]
    className?: string
    /** Show the current and peak figures beside the line. */
    showLabels?: boolean
    height?: number
}

/**
 * The smallest top-of-scale, in bytes per second.
 *
 * 256 KB/s: below that the line is drawing noise, and a graph of noise reads as
 * a broken download rather than a slow one.
 */
const FLOOR_BPS = 256 * 1024

export default function SpeedGraph({
    samples,
    className = '',
    showLabels = false,
    height = 40,
}: Props) {
    const width = 240

    const { area, line, peak, current } = useMemo(() => {
        if (samples.length === 0) return { area: '', line: '', peak: 0, current: 0 }

        const peak = Math.max(...samples)
        const scale = Math.max(peak, FLOOR_BPS)

        // One point per sample, spread across the full width, so a graph with
        // four points and one with two hundred both fill the box.
        const step = samples.length > 1 ? width / (samples.length - 1) : width

        const points = samples.map((value, index) => {
            const x = index * step
            // 2px of headroom, so the peak is not clipped by the stroke.
            const y = height - 2 - (value / scale) * (height - 4)

            return `${x.toFixed(1)},${y.toFixed(1)}`
        })

        const line = `M ${points.join(' L ')}`

        return {
            line,
            area: `${line} L ${width},${height} L 0,${height} Z`,
            peak,
            current: samples[samples.length - 1] ?? 0,
        }
    }, [samples, height])

    if (samples.length === 0)
        return (
            <div
                className={`flex items-center justify-center text-[0.65rem] text-muted ${className}`}
                style={{ height }}
            >
                No data yet
            </div>
        )

    return (
        <div className={`flex items-center gap-2 ${className}`}>
            <svg
                viewBox={`0 0 ${width} ${height}`}
                preserveAspectRatio="none"
                className="h-full min-w-0 flex-1"
                style={{ height }}
                role="img"
                aria-label={`Download speed, currently ${formatSpeed(current)}`}
            >
                {/*
                 * `currentColor` on both, so the graph inherits whatever the
                 * row is coloured — a paused row's line dims with it, and a
                 * theme plugin's palette reaches this with no extra tokens.
                 */}
                <path d={area} fill="currentColor" opacity={0.12} />
                <path
                    d={line}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth={1.5}
                    strokeLinejoin="round"
                    vectorEffect="non-scaling-stroke"
                />
            </svg>

            {showLabels && (
                <div className="shrink-0 text-right text-[0.65rem] leading-tight text-muted">
                    <div className="font-medium text-foreground">
                        {formatSpeed(current)}
                    </div>
                    <div>peak {formatSpeed(peak)}</div>
                </div>
            )}
        </div>
    )
}
