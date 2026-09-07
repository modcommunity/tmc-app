/**
 * A game's artwork, wherever a game is named.
 *
 * The app draws one flat list across every game in the catalogue — a browse
 * grid mixing Minecraft mods and Rust servers, a sandbox list with six games in
 * it, a filter picker naming two hundred. On a site whose chrome is built
 * around one chosen game none of that needs an icon; here it is the difference
 * between scanning a list and reading it.
 *
 * THE FALLBACK IS NOT OPTIONAL
 * ---------------------------
 * `icon` is null for a game with no artwork uploaded, and for every response
 * from a server predating the field. Both are normal, so this always renders
 * something: the game's initials on a tinted square, derived from its name so
 * two games are reliably different colours and one game is the same colour
 * everywhere. A blank space would be worse than no icon at all, because a list
 * of rows where some are indented by an image and some are not reads as broken.
 *
 * The image is never the only thing carrying the name. Every caller here puts
 * the label beside it — an icon on its own is a guess, and a wrong guess about
 * which game a mod is for is how somebody installs it into the wrong folder.
 */

import { appLabel } from '~/lib/api/labels'

/** Sizes that exist, so a caller cannot invent a half-size and break a row. */
const SIZES = {
    sm: 'h-5 w-5 rounded text-[9px]',
    md: 'h-8 w-8 rounded-md text-[11px]',
    lg: 'h-12 w-12 rounded-lg text-sm',
} as const

export type GameIconSize = keyof typeof SIZES

export function GameIcon({
    app,
    size = 'md',
    className = '',
}: {
    app: { id: number; name: string; icon?: string | null } | null | undefined
    size?: GameIconSize
    className?: string
}) {
    const box = `${SIZES[size]} shrink-0 overflow-hidden ${className}`

    if (!app) {
        return (
            <span
                aria-hidden
                className={`${box} border border-dashed border-border`}
            />
        )
    }

    const label = appLabel(app)

    if (app.icon) {
        return (
            <img
                src={app.icon}
                alt=""
                loading="lazy"
                /*
                 * `alt=""` and `aria-hidden`: the game's name is always next to
                 * this in text, so announcing it again makes a screen reader
                 * read every row twice.
                 */
                aria-hidden
                className={`${box} border border-border object-cover`}
            />
        )
    }

    return (
        <span
            aria-hidden
            title={label}
            className={`${box} flex items-center justify-center border border-border font-semibold uppercase tracking-tight text-foreground`}
            style={{ backgroundColor: tint(label) }}
        >
            {initials(label)}
        </span>
    )
}

/**
 * One or two letters from the game's name.
 *
 * Words first — "Grand Theft Auto V" is `GT`, not `GR` — because the initials
 * of separate words are what people actually recognise a title by.
 */
function initials(name: string): string {
    const words = name.split(/[\s:–—-]+/).filter(Boolean)

    if (words.length >= 2) {
        return `${words[0]?.[0] ?? ''}${words[1]?.[0] ?? ''}`
    }

    return name.slice(0, 2)
}

/**
 * A stable colour for a name.
 *
 * Hashed rather than picked from a palette by index: the same game has to get
 * the same colour in the browser, the sandbox list and the filter picker, and
 * those three lists are ordered differently and hold different subsets.
 *
 * Kept dim and desaturated — this sits behind two letters of foreground text in
 * both themes, and a saturated fill is unreadable in one of them whichever one
 * it is tuned for.
 */
function tint(name: string): string {
    let hash = 0

    for (const ch of name) {
        hash = (hash * 31 + ch.charCodeAt(0)) % 360
    }

    return `color-mix(in oklab, hsl(${hash} 70% 50%) 22%, var(--surface-secondary))`
}
