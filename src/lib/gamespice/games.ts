/**
 * Which games are GameSpice's, and how each one looks.
 *
 * TMC lists every game it hosts; GameSpice is the platform's own (the games
 * built on it). The website decides with an allowlist by the game record's url slug
 * (website-gs `src/lib/platform.ts`, `PLATFORM_GAMES`) and gives four of them
 * their own accent and display font (`KNOWN_GAMES`). The app needs the same
 * answer, so this is the mirror of those two tables.
 *
 * It is a copy, and that is a known debt: the list belongs in a shared package
 * both consume (tracked on the nightly board). Until then a game added to
 * website-gs needs its url added here too; `
 * Games screen says so when the list is empty.
 */

/** One game: its url slugs, current one first, then former urls. */
export type PlatformGame = readonly [string, ...string[]]

export const PLATFORM_GAMES: readonly PlatformGame[] = [
    ['prop-hunt'],
    ['playlot', 'playground'],
    ['fatal-contact', 'fatalcontact', 'arena'],
    ['airdrift', 'g2gfast'],
    ['hungario'],
    ['the-room'],
    ['look-at-me'],
    ['hide-n-seek'],
    ['murder-mystery'],
    ['zombie-zone'],
    ['natural-disasters'],
    ['buses-from-hell'],
    ['dangerous-delivery'],
    ['deathrun'],
    ['smash-copter'],
    ['wipeout'],
]

const ALL_SLUGS: ReadonlySet<string> = new Set(PLATFORM_GAMES.flat())

/** Whether a url slug (current or former) is a platform game. */
export function isPlatformSlug(slug: string | null | undefined): boolean {
    return slug != null && ALL_SLUGS.has(slug.toLowerCase())
}

/** The current url of a game given any of its urls; the input when unknown. */
export function canonicalSlug(slug: string): string {
    const lower = slug.toLowerCase()

    return PLATFORM_GAMES.find((g) => g.includes(lower))?.[0] ?? lower
}

/** The display faces a game may use; each maps to a CSS stack in `app.css`. */
export type DisplayFont = 'jakarta' | 'display' | 'mono'

export type GameLook = {
    /** CSS colour for the game's accent. */
    accent: string
    hue: number
    font: DisplayFont
    uppercase: boolean
    name?: string
}

/** The four games with their own look (website-gs `KNOWN_GAMES`). */
const KNOWN: Record<string, GameLook> = {
    'prop-hunt': { accent: '#c026d3', hue: 292, font: 'jakarta', uppercase: false, name: 'Prop Hunt' },
    playlot: { accent: '#84cc16', hue: 150, font: 'jakarta', uppercase: false, name: 'Playlot' },
    'fatal-contact': { accent: '#f97316', hue: 18, font: 'display', uppercase: true, name: 'Fatal Contact' },
    airdrift: { accent: '#06b6d4', hue: 195, font: 'mono', uppercase: true, name: 'Airdrift' },
}

/** FNV-1a, so an unknown game keeps one hue everywhere. */
export function hashOf(text: string): number {
    let h = 0x811c9dc5

    for (let i = 0; i < text.length; i++) {
        h ^= text.charCodeAt(i)
        h = Math.imul(h, 0x01000193) >>> 0
    }

    return h
}

/** A game's look: the known one, else a hue derived from its slug. */
export function gameLook(slug: string): GameLook {
    const key = canonicalSlug(slug)
    const known = KNOWN[key]

    if (known) return known

    const hue = hashOf(key) % 360

    return { accent: `hsl(${hue} 72% 52%)`, hue, font: 'jakarta', uppercase: false }
}

/** Inline style for a game's heading: its display font, case and accent. */
export function gameHeadingStyle(look: GameLook): Record<string, string> {
    return {
        fontFamily: `var(--game-font-${look.font}, var(--font-heading, inherit))`,
        textTransform: look.uppercase ? 'uppercase' : 'none',
        color: look.accent,
    }
}
