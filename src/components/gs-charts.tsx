import { useId, useState } from 'react'

import { dayLabel, sparkPath } from '~/lib/gamespice/stats'

/**
 * The Stats screen's charts: hand-rolled SVG for the same reason as
 * `speed-graph`/`latency-graph` (a handful of points, no axes worth a
 * library). Every mark has a hover read-out and every chart a text summary in
 * its `aria-label`, so nothing is told by colour alone.
 */

/** Days with play over the last 30 days: one bar per day, hover for the date and count. */
export function ActivityBars({
    series,
    since,
    accent = 'var(--color-accent)',
    height = 112,
    unit = 'game',
}: {
    series: readonly number[]
    since: string
    accent?: string
    height?: number
    unit?: string
}) {
    const [hover, setHover] = useState<number | null>(null)
    const max = Math.max(1, ...series)
    const n = series.length
    const gap = 2
    const w = 100 / Math.max(1, n)
    const active = series.filter((v) => v > 0).length
    const shown = hover ?? null
    const plural = (v: number) => `${v} ${unit}${v === 1 ? '' : 's'}`

    return (
        <figure className="flex flex-col gap-1.5" aria-label={`Played on ${active} of the last ${n} days`}>
            <div className="relative" style={{ height }} onMouseLeave={() => setHover(null)}>
                {/* Recessive guide lines at the top and the middle. */}
                <div className="pointer-events-none absolute inset-x-0 top-0 border-t border-dashed border-border/60" />
                <div className="pointer-events-none absolute inset-x-0 top-1/2 border-t border-dashed border-border/40" />
                <div className="absolute inset-0 flex items-end">
                    {series.map((v, i) => (
                        <button
                            key={i}
                            type="button"
                            aria-label={`${dayLabel(since, i)}: ${plural(v)}`}
                            onMouseEnter={() => setHover(i)}
                            onFocus={() => setHover(i)}
                            onBlur={() => setHover(null)}
                            className="group flex h-full items-end outline-none"
                            style={{ width: `${w}%`, paddingInline: gap / 2 }}
                        >
                            <span
                                className="gs-grow block w-full rounded-t-[4px] transition-[opacity,transform] duration-200 group-focus-visible:ring-2 group-focus-visible:ring-accent"
                                style={{
                                    height: v > 0 ? `${Math.max(6, (v / max) * 100)}%` : 3,
                                    background: v > 0 ? accent : 'var(--color-border)',
                                    opacity: shown === null || shown === i ? 1 : 0.45,
                                    animationDelay: `${i * 12}ms`,
                                }}
                            />
                        </button>
                    ))}
                </div>
                {shown !== null && (
                    <div
                        role="status"
                        className="pointer-events-none absolute -top-2 z-10 -translate-x-1/2 -translate-y-full whitespace-nowrap rounded-md border border-border bg-surface px-2 py-1 text-[11px] shadow-lg"
                        style={{ left: `${(shown + 0.5) * w}%` }}
                    >
                        <span className="font-semibold text-foreground">{dayLabel(since, shown)}</span>
                        <span className="ml-1.5 text-muted">{plural(series[shown] ?? 0)}</span>
                    </div>
                )}
            </div>
            <figcaption className="flex justify-between text-[10px] text-muted">
                <span>{dayLabel(since, 0)}</span>
                <span>{dayLabel(since, Math.floor((n - 1) / 2))}</span>
                <span>Today</span>
            </figcaption>
        </figure>
    )
}

/** A small area sparkline for one game's last 30 days. */
export function Sparkline({
    series,
    accent,
    width = 120,
    height = 28,
    label,
}: {
    series: readonly number[]
    accent: string
    width?: number
    height?: number
    label: string
}) {
    const id = useId()
    const { line, area } = sparkPath(series, width, height)

    return (
        <svg
            viewBox={`0 0 ${width} ${height}`}
            width="100%"
            height={height}
            preserveAspectRatio="none"
            role="img"
            aria-label={label}
            className="overflow-visible"
        >
            <defs>
                <linearGradient id={id} x1="0" x2="0" y1="0" y2="1">
                    <stop offset="0%" stopColor={accent} stopOpacity={0.35} />
                    <stop offset="100%" stopColor={accent} stopOpacity={0} />
                </linearGradient>
            </defs>
            <path d={area} fill={`url(#${id})`} />
            <path d={line} fill="none" stroke={accent} strokeWidth={2} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
        </svg>
    )
}

/** A ring showing a percentage, with the figure in the middle. */
export function ProgressRing({
    value,
    accent = 'var(--color-accent)',
    size = 56,
    label,
}: {
    value: number
    accent?: string
    size?: number
    label: string
}) {
    const r = (size - 8) / 2
    const c = 2 * Math.PI * r
    const pct = Math.max(0, Math.min(100, value))

    return (
        <div className="relative shrink-0" style={{ width: size, height: size }} role="img" aria-label={`${label}: ${pct}%`}>
            <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size} className="-rotate-90">
                <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--color-border)" strokeWidth={6} />
                <circle
                    cx={size / 2}
                    cy={size / 2}
                    r={r}
                    fill="none"
                    stroke={accent}
                    strokeWidth={6}
                    strokeLinecap="round"
                    strokeDasharray={c}
                    strokeDashoffset={c * (1 - pct / 100)}
                    className="transition-[stroke-dashoffset] duration-700 ease-out"
                />
            </svg>
            <span className="absolute inset-0 flex items-center justify-center text-xs font-bold text-foreground">{pct}%</span>
        </div>
    )
}

export type ShareSegment = { key: string; label: string; value: number; share: number; color: string }

/** One horizontal bar split by share, with a legend that names each part and its figure. */
export function ShareBar({ segments, unit }: { segments: ShareSegment[]; unit: string }) {
    const [hover, setHover] = useState<string | null>(null)

    return (
        <div className="flex flex-col gap-3">
            <div className="flex h-3 w-full gap-[2px] overflow-hidden rounded-full" role="img" aria-label={segments.map((s) => `${s.label} ${Math.round(s.share * 100)}%`).join(', ')}>
                {segments.map((s) => (
                    <span
                        key={s.key}
                        title={`${s.label}: ${s.value} ${unit}`}
                        onMouseEnter={() => setHover(s.key)}
                        onMouseLeave={() => setHover(null)}
                        className="gs-grow-x h-full transition-opacity first:rounded-l-full last:rounded-r-full"
                        style={{ width: `${s.share * 100}%`, background: s.color, opacity: hover === null || hover === s.key ? 1 : 0.4 }}
                    />
                ))}
            </div>
            <ul className="grid grid-cols-1 gap-x-4 gap-y-1.5 text-xs sm:grid-cols-2">
                {segments.map((s) => (
                    <li
                        key={s.key}
                        className="flex items-center gap-2"
                        onMouseEnter={() => setHover(s.key)}
                        onMouseLeave={() => setHover(null)}
                    >
                        <span className="size-2.5 shrink-0 rounded-sm" style={{ background: s.color }} />
                        <span className="min-w-0 flex-1 truncate text-foreground">{s.label}</span>
                        <span className="tabular-nums text-muted">
                            {s.value} {unit} · {Math.round(s.share * 100)}%
                        </span>
                    </li>
                ))}
            </ul>
        </div>
    )
}
