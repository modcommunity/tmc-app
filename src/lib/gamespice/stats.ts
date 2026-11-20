import type { MeStatsResponseT } from '~/lib/api/contract'

/**
 * The Stats screen's arithmetic: streaks, series, date labels and the SVG
 * paths its charts draw. Pure, so the numbers on screen are tested rather
 * than eyeballed.
 */

const DAY_MS = 86_400_000

/** The UTC date `offset` days after `since` (`YYYY-MM-DD`). */
export function dayAfter(since: string, offset: number): Date {
    return new Date(Date.parse(`${since}T00:00:00Z`) + offset * DAY_MS)
}

/** "Nov 3" for a series index; the API's days are UTC days. */
export function dayLabel(since: string, offset: number): string {
    return dayAfter(since, offset).toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' })
}

/** Consecutive days with play, counting back from the last day of the series (today). */
export function currentStreak(series: readonly number[]): number {
    let n = 0

    for (let i = series.length - 1; i >= 0; i--) {
        if ((series[i] ?? 0) <= 0) {
            // Today not played yet does not break a streak that ran to yesterday.
            if (i === series.length - 1) continue
            break
        }
        n++
    }

    return n
}

/** The longest run of consecutive days with play in the series. */
export function longestStreak(series: readonly number[]): number {
    let best = 0
    let run = 0

    for (const v of series) {
        run = v > 0 ? run + 1 : 0
        best = Math.max(best, run)
    }

    return best
}

/** "today", "yesterday", "5 days ago", or the date for anything older than a month. */
export function lastPlayedLabel(day: string | null, now: Date = new Date()): string {
    if (!day) return 'never'

    const today = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate())
    const diff = Math.round((today - Date.parse(`${day}T00:00:00Z`)) / DAY_MS)

    if (diff <= 0) return 'today'
    if (diff === 1) return 'yesterday'
    if (diff < 31) return `${diff} days ago`

    return new Date(`${day}T00:00:00Z`).toLocaleDateString('en-US', {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        timeZone: 'UTC',
    })
}

/** Unlocked over total as a whole percent; 0 when the game declares none. */
export function completion(ach: { unlocked: number; total: number } | null): number {
    if (!ach || ach.total <= 0) return 0

    return Math.min(100, Math.round((ach.unlocked / ach.total) * 100))
}

/**
 * A polyline/area path for a series in a `w` x `h` box, with `pad` kept clear
 * at the top so a peak does not touch the edge. The y scale is the series'
 * own max with a floor of 1, so an all-zero series is a flat baseline.
 */
export function sparkPath(series: readonly number[], w: number, h: number, pad = 2): { line: string; area: string } {
    if (series.length === 0) return { line: '', area: '' }

    const max = Math.max(1, ...series)
    const step = series.length > 1 ? w / (series.length - 1) : 0
    const pts = series.map((v, i) => [i * step, h - (Math.max(0, v) / max) * (h - pad)] as const)
    const line = pts.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(1)} ${y.toFixed(1)}`).join(' ')

    return { line, area: `${line} L${w.toFixed(1)} ${h} L0 ${h} Z` }
}

/** The games' share of all days played, biggest first, the tail folded into "other" past `top`. */
export function playShare(
    games: MeStatsResponseT['games'],
    top = 5
): { appId: number | null; days: number; share: number }[] {
    const total = games.reduce((n, g) => n + g.daysPlayed, 0)

    if (total <= 0) return []

    const sorted = [...games].sort((a, b) => b.daysPlayed - a.daysPlayed || a.appId - b.appId)
    const head = sorted.slice(0, top).map((g) => ({ appId: g.appId as number | null, days: g.daysPlayed }))
    const rest = sorted.slice(top).reduce((n, g) => n + g.daysPlayed, 0)

    if (rest > 0) head.push({ appId: null, days: rest })

    return head.map((s) => ({ ...s, share: s.days / total }))
}

/** A stat value for display: integers grouped, fractions to two places. */
export function statValue(v: number): string {
    return Number.isInteger(v)
        ? v.toLocaleString('en-US')
        : v.toLocaleString('en-US', { maximumFractionDigits: 2 })
}
