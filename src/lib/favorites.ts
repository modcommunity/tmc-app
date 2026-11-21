import { useSyncExternalStore } from 'react'

import type { ContentSummaryT } from '~/lib/api/contract'

/**
 * Favourite servers, kept on this device.
 *
 * WHY LOCAL: the app API (`/api/app/v1`) has no favourites endpoint, and
 * `/browse` has neither a `favorites` switch nor an `ids` filter. The website
 * stores `Favorite` rows server-side and resolves "Favourites only" there; the
 * app cannot. So favourites live in localStorage as a snapshot of the row
 * (`ContentSummary`), which is also what lets the Favourites view render with no
 * way to ask the API for "these ids". Snapshots are refreshed from the item
 * endpoint when the view opens.
 */

export const FAVORITES_KEY = 'tmc.favorites.servers'
export const MAX_FAVORITES = 500

export type FavoriteMap = Readonly<Record<string, ContentSummaryT>>

/** Pure toggle: add when absent, remove when present. Never mutates. */
export function toggleFavorite(map: FavoriteMap, item: ContentSummaryT): FavoriteMap {
    const id = String(item.id)

    if (id in map) {
        const { [id]: _removed, ...rest } = map

        return rest
    }

    const next = { ...map, [id]: item }
    const keys = Object.keys(next)

    // Oldest-inserted first; drop the overflow rather than refusing to save.
    if (keys.length > MAX_FAVORITES) {
        for (const key of keys.slice(0, keys.length - MAX_FAVORITES)) delete next[key]
    }

    return next
}

/** Replace snapshots for ids that are still favourites; never adds. */
export function refreshFavorites(map: FavoriteMap, fresh: ContentSummaryT[]): FavoriteMap {
    const next: Record<string, ContentSummaryT> = { ...map }

    for (const item of fresh) {
        const id = String(item.id)

        if (id in next) next[id] = item
    }

    return next
}

export type FavoriteFilters = {
    search?: string
    appId?: number
    onlineOnly?: boolean
    hideEmpty?: boolean
    hideFull?: boolean
    password?: boolean
    secure?: boolean
    isOfficial?: boolean
}

/** Client-side equivalent of the API filters the Favourites view can honour. */
export function filterFavorites(
    list: ContentSummaryT[],
    f: FavoriteFilters
): ContentSummaryT[] {
    const needle = f.search?.trim().toLowerCase()

    return list.filter((item) => {
        const s = item.server

        if (needle && !item.name.toLowerCase().includes(needle)) return false
        if (f.appId != null && item.app?.id !== f.appId) return false
        if (f.onlineOnly && s && !s.online) return false
        if (f.isOfficial && !item.isOfficial) return false
        if (f.hideEmpty && s && s.curUsers <= 0) return false
        if (f.hideFull && s && s.maxUsers > 0 && s.curUsers >= s.maxUsers) return false

        return true
    })
}

export function parseFavorites(raw: string | null): FavoriteMap {
    try {
        const value: unknown = JSON.parse(raw ?? '{}')

        if (typeof value !== 'object' || value === null || Array.isArray(value)) return {}

        const out: Record<string, ContentSummaryT> = {}

        for (const [key, item] of Object.entries(value)) {
            if (/^\d+$/.test(key) && typeof item === 'object' && item !== null) {
                out[key] = item as ContentSummaryT
            }
        }

        return out
    } catch {
        return {}
    }
}

// ------------------------------------------------------------------ store

let current: FavoriteMap | null = null
const listeners = new Set<() => void>()

function read(): FavoriteMap {
    if (current === null) {
        let raw: string | null = null

        try {
            raw = typeof localStorage === 'undefined' ? null : localStorage.getItem(FAVORITES_KEY)
        } catch {
            // Blocked storage: start empty rather than crash every render.
        }

        current = parseFavorites(raw)
    }

    return current
}

function write(next: FavoriteMap): void {
    current = next

    try {
        localStorage.setItem(FAVORITES_KEY, JSON.stringify(next))
    } catch {
        // Blocked storage: favourites last until the window closes.
    }

    for (const listener of listeners) listener()
}

export function toggleStoredFavorite(item: ContentSummaryT): void {
    write(toggleFavorite(read(), item))
}

export function refreshStoredFavorites(fresh: ContentSummaryT[]): void {
    write(refreshFavorites(read(), fresh))
}

function subscribe(listener: () => void): () => void {
    listeners.add(listener)

    return () => listeners.delete(listener)
}

/** The favourites map; re-renders on any change (optimistic by construction). */
export function useFavorites(): FavoriteMap {
    return useSyncExternalStore(subscribe, read, read)
}
