import { useQuery } from '@tanstack/react-query'

import { api } from '~/lib/api/client'
import { DEFAULT_FILTERS, pageQuery } from '~/lib/api/play'
import type { PlayAppRowT } from '~/components/play-center'
import { canonicalSlug, isPlatformSlug } from './games'

/**
 * One platform game by slug, from the same `GET /play/page` read the catalogue
 * uses. Resolves to null when the site does not list it (or play is off).
 */
export function useGameBySlug(slug: string | undefined) {
    return useQuery({
        queryKey: ['gamespice', 'game', slug],
        enabled: slug !== undefined && isPlatformSlug(slug),
        queryFn: async (): Promise<PlayAppRowT | null> => {
            let cursor = 0

            for (;;) {
                // No `q`: the site searches NAMES, so a slug (`fatal-contact`)
                // or a former url (`arena`) never matches. The list is short.
                const page = await api.playPage(
                    pageQuery({ ...DEFAULT_FILTERS, tab: 'games' }, cursor)
                )
                const hit = page?.apps.find(
                    (a) =>
                        a.url !== null &&
                        slug !== undefined &&
                        canonicalSlug(a.url) === canonicalSlug(slug)
                )

                if (hit) return hit
                if (!page || page.nextCursor === null) return null

                cursor = page.nextCursor
            }
        },
        staleTime: 60_000,
    })
}

/** Every platform game the site lists (all catalogue pages, filtered to the allowlist). */
export function usePlatformGames() {
    return useQuery({
        queryKey: ['gamespice', 'platform-games'],
        queryFn: async (): Promise<PlayAppRowT[]> => {
            const out: PlayAppRowT[] = []
            let cursor = 0

            for (;;) {
                const page = await api.playPage(
                    pageQuery({ ...DEFAULT_FILTERS, tab: 'games' }, cursor)
                )

                if (!page) return out

                out.push(...page.apps.filter((a) => isPlatformSlug(a.url)))

                if (page.nextCursor === null) return out

                cursor = page.nextCursor
            }
        },
        staleTime: 60_000,
    })
}
