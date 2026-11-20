import { useQuery } from '@tanstack/react-query'

import { api } from '~/lib/api/client'
import { DEFAULT_FILTERS, pageQuery } from '~/lib/api/play'
import type { PlayAppRowT } from '~/components/play-center'
import { isPlatformSlug } from './games'

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
                const page = await api.playPage(
                    pageQuery({ ...DEFAULT_FILTERS, tab: 'games', q: slug ?? '' }, cursor)
                )
                const hit = page?.apps.find((a) => a.url === slug)

                if (hit) return hit
                if (!page || page.nextCursor === null) return null

                cursor = page.nextCursor
            }
        },
        staleTime: 60_000,
    })
}
