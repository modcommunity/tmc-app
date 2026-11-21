/**
 * The merged Play page's game list as pure functions: which rows to drop
 * (engines, Godot), how they dedupe, how the filter chips narrow them and how
 * they split into the GameSpice section and the rest.
 */
import { isPlatformSlug } from './games'

export type CatalogueChip = 'all' | 'multiplayer' | 'singleplayer' | 'gamespice'

export const CHIPS: readonly CatalogueChip[] = ['all', 'multiplayer', 'singleplayer', 'gamespice']

export const CHIP_LABELS: Record<CatalogueChip, string> = {
    all: 'All',
    multiplayer: 'Multiplayer',
    singleplayer: 'Singleplayer',
    gamespice: 'GameSpice',
}

/** The fields of a play-page app row this logic reads. */
export type CatalogueRow = {
    id: number
    name: string
    url: string | null
    solo: boolean
    serverCnt: number
    lobbyCnt: number
    playerCnt: number
    isEngine: boolean
}

/** Engines (Godot included) are tooling, not games, and are not shown on Play. */
export function isHidden(row: CatalogueRow): boolean {
    return row.isEngine || /godot/i.test(row.name) || /godot/i.test(row.url ?? '')
}

export function isPlatform(row: CatalogueRow): boolean {
    return isPlatformSlug(row.url)
}

export function isSingleplayer(row: CatalogueRow): boolean {
    return row.solo
}

export function isMultiplayer(row: CatalogueRow): boolean {
    return !row.solo || row.serverCnt > 0 || row.lobbyCnt > 0
}

/** One row per game id (first wins), engines removed. */
export function mergeGames<T extends CatalogueRow>(rows: readonly T[]): T[] {
    const seen = new Set<number>()
    const out: T[] = []

    for (const row of rows) {
        if (seen.has(row.id) || isHidden(row)) continue
        seen.add(row.id)
        out.push(row)
    }

    return out
}

export function matchesChip(row: CatalogueRow, chip: CatalogueChip): boolean {
    switch (chip) {
        case 'multiplayer':
            return isMultiplayer(row)
        case 'singleplayer':
            return isSingleplayer(row)
        case 'gamespice':
            return isPlatform(row)
        default:
            return true
    }
}

export function filterGames<T extends CatalogueRow>(rows: readonly T[], chip: CatalogueChip, q: string): T[] {
    const needle = q.trim().toLowerCase()

    return rows.filter(
        (r) => matchesChip(r, chip) && (!needle || r.name.toLowerCase().includes(needle))
    )
}

/** Platform games first (busiest first), every other game once, in its given order. */
export function splitSections<T extends CatalogueRow>(rows: readonly T[]): { platform: T[]; others: T[] } {
    const platform = rows
        .filter(isPlatform)
        .sort((a, b) => b.playerCnt - a.playerCnt || a.name.localeCompare(b.name))

    return { platform, others: rows.filter((r) => !isPlatform(r)) }
}
