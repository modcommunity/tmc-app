/**
 * Pure logic behind the monitor window: the rolling history, the statistics
 * over it, window labels and URLs, and the chart scale helpers.
 *
 * Kept free of React and of Tauri so all of it is unit-tested in node. The
 * window itself (`routes/monitor.tsx`) only draws what these return.
 */

// ---------------------------------------------------------------- History

/** One probe of the server. `rtt === null` is a timeout or an error. */
export type MonitorSample = {
    /** Milliseconds since the Unix epoch. */
    at: number
    rtt: number | null
    /** Players at the time, when the protocol reported a count. */
    players: number | null
    maxPlayers: number | null
}

/**
 * A fixed-capacity buffer that forgets its oldest entry first.
 *
 * Backed by a plain array with a moving head rather than `shift()`, which is
 * linear: at one probe a second for a day-long session that cost adds up. The
 * buffer holds mutable state on purpose; the window copies it out with
 * `toArray()` when it renders.
 */
export class RingBuffer<T> {
    private items: (T | undefined)[]
    private head = 0
    private count = 0

    constructor(readonly capacity: number) {
        if (!Number.isInteger(capacity) || capacity < 1)
            throw new RangeError('capacity must be a positive integer')

        this.items = new Array<T | undefined>(capacity)
    }

    get size(): number {
        return this.count
    }

    push(item: T): void {
        const index = (this.head + this.count) % this.capacity

        if (this.count < this.capacity) {
            this.items[index] = item
            this.count++

            return
        }

        // Full: overwrite the oldest and advance the head past it.
        this.items[this.head] = item
        this.head = (this.head + 1) % this.capacity
    }

    /** Oldest first. */
    toArray(): T[] {
        const out: T[] = []

        for (let i = 0; i < this.count; i++)
            out.push(this.items[(this.head + i) % this.capacity] as T)

        return out
    }

    clear(): void {
        this.items = new Array<T | undefined>(this.capacity)
        this.head = 0
        this.count = 0
    }
}

/** Samples kept in a window: 2.7 hours at one probe a second. */
export const HISTORY_CAPACITY = 10_000

/**
 * Merge older samples (the app's own latency history) in front of the
 * window's, dropping any at or after the first live one so nothing doubles.
 */
export function mergeHistory(
    seed: MonitorSample[],
    live: MonitorSample[]
): MonitorSample[] {
    const first = live[0]?.at ?? Infinity

    return [...seed.filter((s) => s.at < first), ...live]
}

// ------------------------------------------------------------------ Ranges

export type RangeId = '1m' | '5m' | '15m' | '1h' | 'session'

export const RANGES: { id: RangeId; label: string; ms: number | null }[] = [
    { id: '1m', label: '1m', ms: 60_000 },
    { id: '5m', label: '5m', ms: 5 * 60_000 },
    { id: '15m', label: '15m', ms: 15 * 60_000 },
    { id: '1h', label: '1h', ms: 60 * 60_000 },
    { id: 'session', label: 'Session', ms: null },
]

export function parseRange(value: unknown): RangeId {
    return RANGES.some((r) => r.id === value) ? (value as RangeId) : '5m'
}

/** The samples inside the range, ending at `now`. `session` keeps everything. */
export function inRange(
    samples: MonitorSample[],
    range: RangeId,
    now: number
): MonitorSample[] {
    const ms = RANGES.find((r) => r.id === range)?.ms ?? null

    if (ms === null) return samples

    const from = now - ms

    return samples.filter((s) => s.at >= from)
}

// -------------------------------------------------------------------- Stats

export type MonitorStats = {
    count: number
    ok: number
    lost: number
    /** 0-100. */
    lossPct: number
    min: number | null
    avg: number | null
    max: number | null
    p95: number | null
    /** Mean absolute difference between consecutive answered probes. */
    jitter: number | null
    last: number | null
}

/** Nearest-rank percentile of an ascending array; `p` in 0-100. */
export function percentile(sorted: number[], p: number): number | null {
    if (sorted.length === 0) return null

    const rank = Math.ceil((Math.min(100, Math.max(0, p)) / 100) * sorted.length)

    return sorted[Math.max(0, rank - 1)] ?? null
}

export function computeStats(samples: MonitorSample[]): MonitorStats {
    const answered = samples
        .map((s) => s.rtt)
        .filter((v): v is number => v !== null)

    const count = samples.length
    const ok = answered.length
    const lost = count - ok

    let jitterSum = 0

    for (let i = 1; i < answered.length; i++)
        jitterSum += Math.abs(answered[i]! - answered[i - 1]!)

    const sorted = [...answered].sort((a, b) => a - b)
    const last = samples.length > 0 ? samples[samples.length - 1]!.rtt : null

    return {
        count,
        ok,
        lost,
        lossPct: count > 0 ? (lost / count) * 100 : 0,
        min: sorted[0] ?? null,
        avg: ok > 0 ? answered.reduce((a, b) => a + b, 0) / ok : null,
        max: sorted[sorted.length - 1] ?? null,
        p95: percentile(sorted, 95),
        jitter: ok > 1 ? jitterSum / (ok - 1) : null,
        last,
    }
}

// ------------------------------------------------------- Labels and URLs

/** What a monitor window is watching. */
export type MonitorTarget =
    | { kind: 'server'; id: number | string; name?: string }
    | {
          kind: 'addr'
          host: string
          port: number
          appId?: number | null
          name?: string
      }

/** 32-bit FNV-1a, in base 36. Stable, so one address is always one window. */
export function hashString(value: string): string {
    let h = 0x811c9dc5

    for (let i = 0; i < value.length; i++) {
        h ^= value.charCodeAt(i)
        h = Math.imul(h, 0x01000193) >>> 0
    }

    return h.toString(36)
}

/** Strip to what a Tauri window label allows: `a-zA-Z0-9-/:_`. */
function safe(part: string): string {
    return part.replace(/[^a-zA-Z0-9]/g, '_')
}

/**
 * The window label. One label per server, so a second click focuses the window
 * that is already open and a different server gets a window of its own. Always
 * starts with `monitor-`, which the capability's `monitor-*` glob matches.
 */
export function monitorLabel(target: MonitorTarget): string {
    if (target.kind === 'server') return `monitor-server-${safe(String(target.id))}`

    const key = `${target.host.trim().toLowerCase()}:${target.port}`

    return `monitor-addr-${safe(key).slice(0, 40)}-${hashString(key)}`
}

/** The hash route the window loads. Relative to the app's `index.html`. */
export function monitorPath(target: MonitorTarget): string {
    if (target.kind === 'server')
        return `/monitor/server/${encodeURIComponent(String(target.id))}`

    const q = new URLSearchParams({
        host: target.host,
        port: String(target.port),
    })

    if (target.appId != null) q.set('app', String(target.appId))
    if (target.name) q.set('name', target.name)

    return `/monitor/addr?${q.toString()}`
}

export function monitorUrl(target: MonitorTarget): string {
    return `index.html#${monitorPath(target)}`
}

export const MONITOR_SIZE = {
    width: 900,
    height: 600,
    minWidth: 420,
    minHeight: 300,
} as const

// ------------------------------------------------------------ Layout mode

export type LayoutMode = 'graph' | 'split' | 'rcon'

export const LAYOUT_KEY = 'tmc.monitor.layout'

export function parseLayoutMode(value: unknown): LayoutMode {
    return value === 'graph' || value === 'split' || value === 'rcon'
        ? value
        : 'graph'
}

// ------------------------------------------------------------ Chart scale

/** Round `value` up to 1, 2, 2.5, 5 or 10 times a power of ten. */
export function niceCeil(value: number): number {
    if (!Number.isFinite(value) || value <= 0) return 1

    const exp = Math.floor(Math.log10(value))
    const base = 10 ** exp
    const f = value / base

    const step = f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10

    return step * base
}

/**
 * Y-axis ticks from 0 to a nice ceiling over `max`. `floor` keeps a perfectly
 * steady server from drawing as a mountain range.
 */
export function yTicks(
    max: number,
    target: number,
    floor: number
): { top: number; ticks: number[] } {
    const top = niceCeil(Math.max(max * 1.1, floor))
    const step = niceCeil(top / Math.max(1, target))
    const ticks: number[] = []

    for (let v = 0; v <= top + step * 1e-9; v += step)
        ticks.push(Math.round(v * 1000) / 1000)

    return { top, ticks }
}

/** Evenly spaced times between `from` and `to`, with `count` intervals. */
export function xTicks(from: number, to: number, count: number): number[] {
    if (to <= from || count < 1) return [from]

    const out: number[] = []

    for (let i = 0; i <= count; i++) out.push(from + ((to - from) * i) / count)

    return out
}

/** Map `v` from the domain onto the range. A degenerate domain maps to the range's start. */
export function scaleLinear(
    v: number,
    d0: number,
    d1: number,
    r0: number,
    r1: number
): number {
    if (d1 === d0) return r0

    return r0 + ((v - d0) / (d1 - d0)) * (r1 - r0)
}

export type Pt = { x: number; y: number }

/**
 * A smooth path through the points using a monotone cubic (Fritsch-Carlson),
 * which never overshoots between samples: a smoothed ping line that dips
 * below zero or above the real maximum would be a lie.
 */
export function smoothPath(points: Pt[]): string {
    const n = points.length

    if (n === 0) return ''

    const f = (v: number) => v.toFixed(2)

    if (n === 1) return `M ${f(points[0]!.x)} ${f(points[0]!.y)}`

    if (n === 2)
        return `M ${f(points[0]!.x)} ${f(points[0]!.y)} L ${f(points[1]!.x)} ${f(points[1]!.y)}`

    const dx: number[] = []
    const slope: number[] = []

    for (let i = 0; i < n - 1; i++) {
        const w = points[i + 1]!.x - points[i]!.x

        dx.push(w)
        slope.push(w === 0 ? 0 : (points[i + 1]!.y - points[i]!.y) / w)
    }

    const m: number[] = [slope[0]!]

    for (let i = 1; i < n - 1; i++) {
        const a = slope[i - 1]!
        const b = slope[i]!

        m.push(a * b <= 0 ? 0 : (a + b) / 2)
    }

    m.push(slope[n - 2]!)

    for (let i = 0; i < n - 1; i++) {
        const s = slope[i]!

        if (s === 0) {
            m[i] = 0
            m[i + 1] = 0
            continue
        }

        const a = m[i]! / s
        const b = m[i + 1]! / s
        const h = Math.hypot(a, b)

        if (h > 3) {
            m[i] = (3 * a * s) / h
            m[i + 1] = (3 * b * s) / h
        }
    }

    let d = `M ${f(points[0]!.x)} ${f(points[0]!.y)}`

    for (let i = 0; i < n - 1; i++) {
        const p0 = points[i]!
        const p1 = points[i + 1]!
        const w = dx[i]! / 3

        d += ` C ${f(p0.x + w)} ${f(p0.y + m[i]! * w)} ${f(p1.x - w)} ${f(p1.y - m[i + 1]! * w)} ${f(p1.x)} ${f(p1.y)}`
    }

    return d
}

/** Index of the entry in ascending `xs` nearest `x`, or -1 for an empty list. */
export function nearestIndex(xs: number[], x: number): number {
    if (xs.length === 0) return -1

    let lo = 0
    let hi = xs.length - 1

    while (hi - lo > 1) {
        const mid = (lo + hi) >> 1

        if (xs[mid]! < x) lo = mid
        else hi = mid
    }

    return Math.abs(xs[lo]! - x) <= Math.abs(xs[hi]! - x) ? lo : hi
}

/** `42s`, `1m 05s`, `1h 02m` — for the session clock. */
export function formatSpan(ms: number): string {
    const s = Math.max(0, Math.round(ms / 1000))

    if (s < 60) return `${s}s`

    const m = Math.floor(s / 60)

    if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`

    return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`
}

/** `-30s` / `-2m` / `now`, the age of a tick relative to the chart's right edge. */
export function formatAgo(ms: number): string {
    if (ms < 500) return 'now'

    const s = Math.round(ms / 1000)

    if (s < 60) return `-${s}s`

    const m = Math.round(s / 60)

    if (m < 60) return `-${m}m`

    return `-${(m / 60).toFixed(m % 60 === 0 ? 0 : 1)}h`
}
