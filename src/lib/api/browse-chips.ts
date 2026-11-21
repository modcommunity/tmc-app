/**
 * The removable chips under the server toolbar, derived from the URL.
 *
 * Pure so the "which filters are on, and what do I call them" rule is tested
 * rather than closed over the route. Each chip names the URL key(s) that
 * removing it deletes.
 */

export type ChipT = { keys: string[]; label: string }

export type ChipNames = {
    apps?: ReadonlyMap<string, string>
    countries?: ReadonlyMap<string, string>
}

const FLAGS: Record<string, string> = {
    wasOnline: 'Seen recently',
    password: 'Password',
    secure: 'Anti-cheat',
    official: 'Official',
    empty: 'Hide empty',
    full: 'Hide full',
    archived: 'Archived',
    mine: 'Mine',
}

const OS: Record<string, string> = { WINDOWS: 'Windows', LINUX: 'Linux', MAC: 'macOS' }
const AGES: Record<string, string> = {
    NONE: 'No age limit',
    ADULT_18: '18+',
    ADULT_21: '21+',
    ADULT_25: '25+',
}

function range(search: URLSearchParams, label: string, lo: string, hi: string): ChipT | null {
    const a = search.get(lo)
    const b = search.get(hi)

    if (!a && !b) return null

    const text = a && b ? `${a}-${b}` : a ? `${a}+` : `up to ${b ?? ''}`

    return { keys: [lo, hi], label: `${label}: ${text}` }
}

export function activeChips(
    search: URLSearchParams,
    names: ChipNames = {},
    opts: { favorites?: boolean } = {}
): ChipT[] {
    const out: ChipT[] = []
    const q = search.get('q')

    if (q) out.push({ keys: ['q'], label: `"${q}"` })

    const app = search.get('app')

    if (app) out.push({ keys: ['app'], label: names.apps?.get(app) ?? `Game ${app}` })

    for (const id of (search.get('countries') ?? '').split(',').filter(Boolean)) {
        out.push({ keys: [`countries:${id}`], label: names.countries?.get(id) ?? `Country ${id}` })
    }

    for (const [key, label] of Object.entries(FLAGS)) {
        if (search.get(key) === '1') out.push({ keys: [key], label })
    }

    // Online is on by default for the listing, and off by default for favourites.
    if (opts.favorites ? search.get('online') === '1' : search.get('online') === '0') {
        out.push({ keys: ['online'], label: opts.favorites ? 'Online only' : 'Including offline' })
    }

    const os = search.get('os')

    if (os) out.push({ keys: ['os'], label: OS[os] ?? os })

    const map = search.get('map')

    if (map) out.push({ keys: ['map'], label: `Map: ${map}` })

    for (const age of (search.get('ages') ?? '').split(',').filter(Boolean)) {
        out.push({ keys: [`ages:${age}`], label: AGES[age] ?? age })
    }

    for (const chip of [
        range(search, 'Players', 'minUsers', 'maxUsers'),
        range(search, 'Slots', 'minSlots', 'maxSlots'),
    ]) {
        if (chip) out.push(chip)
    }

    return out
}

/**
 * Apply a chip's removal to a copy of the params. `key:value` entries remove a
 * single value from a comma list.
 */
export function removeChip(search: URLSearchParams, chip: ChipT): URLSearchParams {
    const next = new URLSearchParams(search)

    for (const key of chip.keys) {
        const [name, value] = key.split(':')

        if (name === undefined) continue

        if (value === undefined) {
            next.delete(name)
            continue
        }

        const rest = (next.get(name) ?? '').split(',').filter((v) => v && v !== value)

        if (rest.length > 0) next.set(name, rest.join(','))
        else next.delete(name)
    }

    return next
}
