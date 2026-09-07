/**
 * Game artwork for screens whose data is local.
 *
 * A sandbox row comes out of this device's SQLite and carries an app id, a
 * name and a slug — no image, and deliberately so: the local database mirrors
 * what the user owns, and caching a CDN URL in it would mean a stale artwork
 * link surviving every sync that changed it.
 *
 * So the icon is looked up instead. `/facets` already returns every game with
 * its artwork, the browse filters already fetch it, and React Query dedupes
 * the two by key — so a sandbox list showing six games costs no extra request
 * on a screen the user reached through the browser, and one request otherwise.
 *
 * `kind` is `mod` because the facet list's APPS do not depend on it — every
 * game in the catalogue is returned whichever content kind is asked about, and
 * pinning one keeps this sharing a cache entry with the mod browser rather than
 * minting a new one per screen.
 */

import { useQuery } from '@tanstack/react-query'

import { api } from '~/lib/api/client'

/** An hour. Artwork changes when somebody re-uploads it, which is rare. */
const STALE_MS = 60 * 60 * 1000

export function useAppIcons(): (appId: number | null | undefined) => string | null {
    const facets = useQuery({
        queryKey: ['facets', 'mod'],
        queryFn: () => api.facets('mod'),
        staleTime: STALE_MS,
    })

    return (appId) => {
        if (appId === null || appId === undefined) return null

        return facets.data?.apps.find((a) => a.id === appId)?.icon ?? null
    }
}

/**
 * The whole reference for a game — id, name and artwork — from the same cache.
 *
 * `useAppIcons` answers half the question, and every caller that needs the
 * other half was pulling the name off a local row instead. That works for a
 * sandbox (which stores `appName`) and not for a game the user has only pointed
 * a FOLDER at: `settings.gameDirs` is keyed by app id and carries nothing else,
 * so the Library's game list would have had rows reading "Game 271590".
 *
 * Same query key as above, so the two share one request.
 */
export function useAppRefs(): (appId: number | null | undefined) => {
    id: number
    name: string
    icon: string | null
    /** The URL slug, which is also the game's plugin folder name. */
    slug: string | null
} | null {
    const facets = useQuery({
        queryKey: ['facets', 'mod'],
        queryFn: () => api.facets('mod'),
        staleTime: STALE_MS,
    })

    return (appId) => {
        if (appId === null || appId === undefined) return null

        const found = facets.data?.apps.find((a) => a.id === appId)

        if (!found) return null

        return {
            id: found.id,
            name: found.name,
            icon: found.icon,
            /*
             * Lower-cased, because a plugin folder is `plugins/app/<slug>` and
             * the app lower-cases every folder name it loads. An operator who
             * typed `GTAV` in the admin form would otherwise produce a slug
             * that matches no rule.
             */
            slug: found.url?.toLowerCase() ?? null,
        }
    }
}
