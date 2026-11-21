/**
 * The play center's filters: what the URL says, what the API is asked, and the
 * small facts every row computes the same way.
 *
 * The screen keeps its filters in the URL for the reason the browse screen does
 * (a HashRouter over a static bundle has nowhere else durable), so the
 * decode lives here, built on `query-params.ts`, and is pure and testable.
 *
 * The tab-to-sort table mirrors website-city's `PlayTabSorts`. It is a copy and
 * not in the contract because it is UI vocabulary: which sorts a tab OFFERS. The
 * server resolves an unsupported sort to the tab's default on its own, so a
 * drift here shows a control that does nothing, never a wrong list.
 */

import {
    PlayModeVals,
    PlaySortVals,
    PlayTabVals,
    type PlayModeT,
    type PlayPageQueryT,
} from './contract'
import { flag, num, oneOf, text } from './query-params'

export type PlayTab = (typeof PlayTabVals)[number]
export type PlaySort = (typeof PlaySortVals)[number]

export const TAB_LABELS: Record<PlayTab, string> = {
    games: 'Games',
    servers: 'Servers',
    lobbies: 'Lobbies',
    invites: 'Invites',
}

export const SORT_LABELS: Record<PlaySort, string> = {
    players: 'Most players',
    recent: 'Recently active',
    name: 'A to Z',
    servers: 'Most servers',
    popular: 'Most popular',
    trending: 'Trending',
    newest: 'Newest',
}

export const MODE_LABELS: Record<PlayModeT, string> = {
    web: 'In a window',
    app: 'Installed game',
}

/** Which sorts each tab can express. Invites is a short list in arrival order. */
export const TAB_SORTS: Record<PlayTab, readonly PlaySort[]> = {
    games: ['players', 'popular', 'trending', 'servers', 'newest', 'name'],
    servers: ['players', 'recent', 'name'],
    lobbies: ['players', 'recent', 'name'],
    invites: [],
}

export const TAB_DEFAULT_SORT: Record<PlayTab, PlaySort> = {
    games: 'players',
    servers: 'players',
    lobbies: 'recent',
    invites: 'recent',
}

export function resolveSort(tab: PlayTab, want: PlaySort | undefined): PlaySort {
    return want && TAB_SORTS[tab].includes(want) ? want : TAB_DEFAULT_SORT[tab]
}

export type PlayFilters = {
    tab: PlayTab
    q: string
    appId?: number
    mode?: PlayModeT
    sort?: PlaySort
    /** Servers only: somebody is on. */
    populated: boolean
    /** Lobbies only: still gathering, not already in a match. */
    joinable: boolean
}

export const DEFAULT_FILTERS: PlayFilters = {
    tab: 'games',
    q: '',
    populated: false,
    joinable: false,
}

/** Decode the URL. Anything unrecognised drops that one filter, never the page. */
export function filtersFromParams(search: URLSearchParams): PlayFilters {
    const appId = num(search, 'app')

    return {
        tab: oneOf(search, 'tab', PlayTabVals) ?? 'games',
        q: (text(search, 'q') ?? '').slice(0, 120),
        appId:
            appId !== undefined && Number.isInteger(appId) && appId > 0
                ? appId
                : undefined,
        mode: oneOf(search, 'mode', PlayModeVals),
        sort: oneOf(search, 'sort', PlaySortVals),
        populated: flag(search, 'populated') === true,
        joinable: flag(search, 'joinable') === true,
    }
}

/**
 * Write the filters back, leaving out everything that is the default so the
 * address stays short and two equal views share one cache entry.
 */
export function filtersToParams(filters: PlayFilters): URLSearchParams {
    const out = new URLSearchParams()

    if (filters.tab !== 'games') out.set('tab', filters.tab)
    if (filters.q) out.set('q', filters.q)
    if (filters.appId) out.set('app', String(filters.appId))
    if (filters.mode) out.set('mode', filters.mode)
    if (filters.sort && filters.sort !== TAB_DEFAULT_SORT[filters.tab])
        out.set('sort', filters.sort)
    if (filters.populated) out.set('populated', '1')
    if (filters.joinable) out.set('joinable', '1')

    return out
}

/**
 * The request for one page.
 *
 * `populated` belongs to servers and `joinable` to lobbies, as on the site; the
 * other tab never sends them, so switching tab cannot leave a switch quietly
 * narrowing a list that has no such control. A sort the tab cannot express is
 * left out for the same reason. The invites tab takes no filters at all.
 */
export function pageQuery(filters: PlayFilters, cursor = 0): PlayPageQueryT {
    const { tab } = filters

    if (tab === 'invites') return { tab }

    const sort =
        filters.sort && TAB_SORTS[tab].includes(filters.sort)
            ? filters.sort
            : undefined

    return {
        tab,
        q: filters.q || undefined,
        // A game narrows nothing on the games tab: it IS the list.
        appId: tab === 'games' ? undefined : filters.appId,
        mode: filters.mode,
        sort,
        populated: tab === 'servers' && filters.populated ? true : undefined,
        joinable: tab === 'lobbies' && filters.joinable ? true : undefined,
        cursor: cursor > 0 ? cursor : undefined,
    }
}

/** True when anything beyond the tab narrows the list: "nothing matched" versus "nothing exists". */
export function isNarrowed(filters: PlayFilters): boolean {
    return Boolean(
        filters.q ||
        filters.appId ||
        filters.mode ||
        filters.populated ||
        filters.joinable
    )
}

/** 0 to 1, or null when the server declares no limit. */
export function fill(cur: number, max: number): number | null {
    if (max <= 0) return null

    return Math.min(1, Math.max(0, cur / max))
}

export function compact(n: number): string {
    if (n < 1000) return String(n)
    if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`

    return `${(n / 1_000_000).toFixed(1)}m`
}

/** "3/16" or "3" when the server declares no limit. */
export function players(cur: number, max: number): string {
    return max > 0 ? `${cur}/${max}` : String(cur)
}
