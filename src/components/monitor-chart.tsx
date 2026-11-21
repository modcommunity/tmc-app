import { useEffect, useId, useMemo, useRef, useState } from 'react'

import {
    nearestIndex,
    formatAgo,
    scaleLinear,
    smoothPath,
    xTicks,
    yTicks,
    type MonitorSample,
    type Pt,
} from '~/lib/monitor'

const MARGIN = { top: 10, right: 12, bottom: 22, left: 40 }

/** Track an element's content box, so the chart redraws crisply at any size. */
function useSize<T extends HTMLElement>() {
    const ref = useRef<T>(null)
    const [size, setSize] = useState({ w: 0, h: 0 })

    useEffect(() => {
        const el = ref.current

        if (!el) return

        const ro = new ResizeObserver(([entry]) => {
            if (!entry) return

            const { width, height } = entry.contentRect

            setSize((prev) =>
                Math.abs(prev.w - width) < 0.5 && Math.abs(prev.h - height) < 0.5
                    ? prev
                    : { w: width, h: height }
            )
        })

        ro.observe(el)

        return () => ro.disconnect()
    }, [])

    return [ref, size] as const
}

/**
 * A time series drawn as SVG that fills whatever box it is given.
 *
 * SVG rather than canvas: it is resolution independent, so it stays sharp on
 * any display scale with no devicePixelRatio bookkeeping, and the handful of
 * paths per chart is cheap. The viewBox is the measured pixel size, so one
 * unit is one CSS pixel and text is never scaled.
 */
export default function TimeChart({
    samples,
    value,
    t0,
    t1,
    color,
    hoverAt,
    onHover,
    floor,
    unit,
    label,
    timeouts = false,
    integer = false,
    tooltip,
    step = false,
}: {
    samples: MonitorSample[]
    value: (s: MonitorSample) => number | null
    t0: number
    t1: number
    color: string
    /** Timestamp of the sample the pointer is over, shared between charts. */
    hoverAt: number | null
    onHover: (at: number | null) => void
    floor: number
    unit: string
    label: string
    timeouts?: boolean
    integer?: boolean
    tooltip: (s: MonitorSample) => string[]
    step?: boolean
}) {
    const [ref, { w, h }] = useSize<HTMLDivElement>()
    const gid = useId().replace(/:/g, '')

    const iw = Math.max(0, w - MARGIN.left - MARGIN.right)
    const ih = Math.max(0, h - MARGIN.top - MARGIN.bottom)

    let max = 0

    for (const s of samples) {
        const v = value(s)

        if (v !== null && v > max) max = v
    }

    const { top, ticks } = yTicks(
        max,
        Math.max(2, Math.min(6, Math.floor(ih / 44))),
        integer ? Math.max(floor, 4) : floor
    )
    const yTickList = integer ? ticks.filter((t) => Number.isInteger(t)) : ticks

    const x = (at: number) => scaleLinear(at, t0, t1, MARGIN.left, MARGIN.left + iw)
    const y = (v: number) => scaleLinear(v, 0, top, MARGIN.top + ih, MARGIN.top)
    const base = MARGIN.top + ih

    // Paths are the expensive part (a long session is thousands of points), and
    // a pointer move re-renders the chart for the crosshair alone, so they are
    // built once per change of data or size rather than once per move.
    const paths = useMemo(() => {
        const xs = (at: number) => scaleLinear(at, t0, t1, MARGIN.left, MARGIN.left + iw)
        const ys = (v: number) => scaleLinear(v, 0, top, MARGIN.top + ih, MARGIN.top)
        const floorY = MARGIN.top + ih

        // Break the line wherever a probe went unanswered.
        const segments: Pt[][] = []
        let run: Pt[] = []

        for (const s of samples) {
            const v = value(s)

            if (v === null) {
                if (run.length) segments.push(run)
                run = []
                continue
            }

            run.push({ x: xs(s.at), y: ys(v) })
        }

        if (run.length) segments.push(run)

        const stepped = (pts: Pt[]) =>
            pts
                .map((p, i) => {
                    const prev = pts[i - 1]

                    return i === 0
                        ? `M ${p.x.toFixed(2)} ${p.y.toFixed(2)}`
                        : `L ${p.x.toFixed(2)} ${prev!.y.toFixed(2)} L ${p.x.toFixed(2)} ${p.y.toFixed(2)}`
                })
                .join(' ')

        return segments.map((pts) => {
            const d = step ? stepped(pts) : smoothPath(pts)
            const first = pts[0]!
            const last = pts[pts.length - 1]!

            return {
                key: first.x,
                single: pts.length === 1,
                first,
                line: d,
                area: `${d} L ${last.x.toFixed(2)} ${floorY} L ${first.x.toFixed(2)} ${floorY} Z`,
            }
        })
        // `value` is a fresh closure each render but always the same accessor.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [samples, t0, t1, iw, ih, top, step])

    const hoverIdx =
        hoverAt === null
            ? -1
            : nearestIndex(
                  samples.map((s) => s.at),
                  hoverAt
              )
    const hovered = hoverIdx >= 0 ? samples[hoverIdx]! : null
    const hoveredValue = hovered ? value(hovered) : null
    const hx = hovered ? x(hovered.at) : 0

    const tipLines = hovered ? tooltip(hovered) : []
    const tipW = 132
    const tipH = 12 + tipLines.length * 15
    const tipLeft = hx + 12 + tipW > w - 4 ? hx - 12 - tipW : hx + 12

    const hasData = paths.length > 0 || samples.some((s) => value(s) === null)

    const description = (() => {
        const vals = samples.map(value).filter((v): v is number => v !== null)

        if (!vals.length) return `${label}: no data yet`

        return `${label}: ${vals.length} samples, latest ${vals[vals.length - 1]}${unit}, range ${Math.min(...vals)} to ${Math.max(...vals)}${unit}`
    })()

    return (
        <div ref={ref} className="relative size-full min-h-0 min-w-0">
            {w > 0 && h > 0 && (
                <svg
                    width={w}
                    height={h}
                    viewBox={`0 0 ${w} ${h}`}
                    role="img"
                    aria-label={description}
                    className="block touch-none select-none"
                    onPointerMove={(e) => {
                        const rect = e.currentTarget.getBoundingClientRect()
                        const px = e.clientX - rect.left

                        if (px < MARGIN.left || px > MARGIN.left + iw) {
                            onHover(null)

                            return
                        }

                        onHover(scaleLinear(px, MARGIN.left, MARGIN.left + iw, t0, t1))
                    }}
                    onPointerLeave={() => onHover(null)}
                >
                    <defs>
                        <linearGradient id={`fill-${gid}`} x1="0" y1="0" x2="0" y2="1">
                            <stop offset="0%" stopColor={color} stopOpacity="0.38" />
                            <stop offset="100%" stopColor={color} stopOpacity="0.02" />
                        </linearGradient>
                    </defs>

                    {/* Grid and y labels */}
                    {yTickList.map((t) => (
                        <g key={t}>
                            <line
                                x1={MARGIN.left}
                                x2={MARGIN.left + iw}
                                y1={y(t)}
                                y2={y(t)}
                                stroke="var(--border)"
                                strokeOpacity={t === 0 ? 1 : 0.55}
                                strokeDasharray={t === 0 ? undefined : '3 4'}
                            />
                            <text
                                x={MARGIN.left - 6}
                                y={y(t)}
                                textAnchor="end"
                                dominantBaseline="central"
                                fontSize="10"
                                fill="var(--muted)"
                            >
                                {t}
                            </text>
                        </g>
                    ))}

                    {/* X labels */}
                    {xTicks(t0, t1, iw < 420 ? 3 : 6).map((t, i, all) => (
                        <text
                            key={t}
                            x={x(t)}
                            y={base + 14}
                            textAnchor={i === 0 ? 'start' : i === all.length - 1 ? 'end' : 'middle'}
                            fontSize="10"
                            fill="var(--muted)"
                        >
                            {formatAgo(t1 - t)}
                        </text>
                    ))}

                    {/* Unanswered probes */}
                    {timeouts &&
                        samples
                            .filter((s) => s.rtt === null)
                            .map((s) => (
                                <rect
                                    key={s.at}
                                    x={x(s.at) - 1.5}
                                    y={MARGIN.top}
                                    width={3}
                                    height={ih}
                                    fill="var(--lat-dead)"
                                    opacity={0.35}
                                />
                            ))}

                    {/* Series */}
                    {paths.map((p) => (
                        <g key={p.key}>
                            {p.single ? (
                                <circle cx={p.first.x} cy={p.first.y} r={2.5} fill={color} />
                            ) : (
                                <>
                                    <path d={p.area} fill={`url(#fill-${gid})`} />
                                    <path
                                        d={p.line}
                                        fill="none"
                                        stroke={color}
                                        strokeWidth={1.75}
                                        strokeLinejoin="round"
                                        strokeLinecap="round"
                                    />
                                </>
                            )}
                        </g>
                    ))}

                    {!hasData && (
                        <text
                            x={MARGIN.left + iw / 2}
                            y={MARGIN.top + ih / 2}
                            textAnchor="middle"
                            fontSize="12"
                            fill="var(--muted)"
                        >
                            Waiting for the first probe…
                        </text>
                    )}

                    {/* Crosshair */}
                    {hovered && (
                        <g pointerEvents="none">
                            <line
                                x1={hx}
                                x2={hx}
                                y1={MARGIN.top}
                                y2={base}
                                stroke="var(--foreground)"
                                strokeOpacity={0.35}
                            />
                            {hoveredValue !== null && (
                                <circle
                                    cx={hx}
                                    cy={y(hoveredValue)}
                                    r={4}
                                    fill={color}
                                    stroke="var(--background)"
                                    strokeWidth={2}
                                />
                            )}
                            <g
                                transform={`translate(${Math.max(4, tipLeft)}, ${MARGIN.top + 4})`}
                            >
                                <rect
                                    width={tipW}
                                    height={tipH}
                                    rx={6}
                                    fill="var(--surface)"
                                    stroke="var(--border)"
                                />
                                {tipLines.map((l, i) => (
                                    <text
                                        key={i}
                                        x={8}
                                        y={16 + i * 15}
                                        fontSize="11"
                                        fontWeight={i === 0 ? 600 : 400}
                                        fill={i === 0 ? 'var(--foreground)' : 'var(--muted)'}
                                    >
                                        {l}
                                    </text>
                                ))}
                            </g>
                        </g>
                    )}
                </svg>
            )}
        </div>
    )
}
