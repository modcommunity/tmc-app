import SectionCards from '~/components/section-cards'
import { DISCOVER } from '~/lib/nav'

/**
 * **Discover** — the landing page for everything the site holds.
 *
 * Section cards only, with no live items per card. Each section's own browse
 * screen already has the filters, paging and offline cache; a second, cut-down
 * copy of those queries here would be five more requests on the screen people
 * open most, to preview what one click shows properly.
 */
export default function DiscoverRoute() {
    return (
        <div className="flex flex-col gap-4 p-4">
            <header>
                <h1 className="text-lg font-bold">Discover</h1>
                <p className="text-xs text-muted">
                    Browse what GameSpice hosts.
                </p>
            </header>

            <SectionCards node={DISCOVER} />
        </div>
    )
}
