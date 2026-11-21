import { FiStar } from 'react-icons/fi'

import type { ContentSummaryT } from '~/lib/api/contract'
import { toggleStoredFavorite, useFavorites } from '~/lib/favorites'

/**
 * The favourite toggle for a server row or card.
 *
 * Optimistic by construction: favourites are a local store (see
 * `lib/favorites`), so the star flips in the same frame as the click. Stops
 * propagation because it sits inside clickable rows and links.
 */
export default function FavoriteStar({
    item,
    className = '',
}: {
    item: ContentSummaryT
    className?: string
}) {
    const favorites = useFavorites()
    const on = String(item.id) in favorites
    const label = on
        ? `Remove ${item.name} from favourites`
        : `Add ${item.name} to favourites`

    return (
        <button
            type="button"
            aria-pressed={on}
            aria-label={label}
            title={on ? 'Favourited' : 'Favourite'}
            onClick={(e) => {
                e.preventDefault()
                e.stopPropagation()
                toggleStoredFavorite(item)
            }}
            className={`rounded-md p-1 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent ${
                on ? 'text-warning' : 'text-muted hover:text-foreground'
            } ${className}`}
        >
            <FiStar className={`size-4 ${on ? 'fill-current' : ''}`} />
        </button>
    )
}
