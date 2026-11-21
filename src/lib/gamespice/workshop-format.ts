/** Pure display helpers for the Workshop cards. */

/** 1234 -> "1.2K", 2_500_000 -> "2.5M"; small numbers stay whole. */
export function compactCount(n: number): string {
    if (!Number.isFinite(n) || n < 0) return '0'
    if (n < 1000) return String(Math.floor(n))
    if (n < 1_000_000) return `${trim(n / 1000)}K`
    if (n < 1_000_000_000) return `${trim(n / 1_000_000)}M`

    return `${trim(n / 1_000_000_000)}B`
}

function trim(v: number): string {
    // One decimal below 10, none above; never "1.0K".
    const s = v < 10 ? v.toFixed(1) : String(Math.floor(v))

    return s.endsWith('.0') ? s.slice(0, -2) : s
}

/** A rating as "4.3", or null when nothing is scored. */
export function formatRating(rating: number | null | undefined): string | null {
    if (rating == null || !Number.isFinite(rating) || rating <= 0) return null

    return Math.min(5, rating).toFixed(1)
}

/** How many of five stars to fill (rounded to the nearest whole star). */
export function filledStars(rating: number | null | undefined): number {
    if (rating == null || !Number.isFinite(rating)) return 0

    return Math.max(0, Math.min(5, Math.round(rating)))
}

/** Up to two initials for an avatar fallback. */
export function initials(name: string | null | undefined): string {
    const parts = (name ?? '').trim().split(/\s+/).filter(Boolean)
    if (parts.length === 0) return '?'
    if (parts.length === 1) return parts[0]!.slice(0, 2).toUpperCase()

    return (parts[0]![0]! + parts[1]![0]!).toUpperCase()
}

/** A stable hue (0-359) for a string, so a kind always gets the same colour. */
export function hueOf(text: string): number {
    let h = 0
    for (const ch of text) h = (h * 31 + ch.codePointAt(0)!) >>> 0

    return h % 360
}

/** CSS gradient for a banner placeholder, coloured by the pack's first kind. */
export function kindGradient(kind: string | null | undefined): string {
    const h = hueOf(kind || 'pack')

    return `linear-gradient(135deg, hsl(${h} 55% 28%), hsl(${(h + 40) % 360} 60% 16%))`
}

/** "today", "3d ago", "2mo ago", "1y ago"; empty for an unreadable date. */
export function relativeDate(
    iso: string | null | undefined,
    now: number = Date.now()
): string {
    const t = iso ? Date.parse(iso) : NaN
    if (Number.isNaN(t)) return ''
    const days = Math.floor((now - t) / 86_400_000)
    if (days < 1) return 'today'
    if (days < 30) return `${days}d ago`
    if (days < 365) return `${Math.floor(days / 30)}mo ago`

    return `${Math.floor(days / 365)}y ago`
}
