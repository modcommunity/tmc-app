import type { z } from 'zod'

import type { WorkshopMineItemSchema, WorkshopOwnedItemSchema } from '~/lib/api/contract'

type MineItemT = z.infer<typeof WorkshopMineItemSchema>
type OwnedItemT = z.infer<typeof WorkshopOwnedItemSchema>

/**
 * What a member reads for a subscription that loads nothing. The site sends
 * the reason (`unavailable`); an older site sends only `available: false`,
 * which is still worth a sentence rather than a bare word.
 */
export function unavailableLabel(item: Pick<MineItemT, 'available' | 'unavailable'>): string | null {
    switch (item.unavailable) {
        case 'missing':
            return 'This pack was deleted'
        case 'hidden':
            return 'Hidden by its owner or a moderator'
        case 'disabled':
            return 'Its owner turned subscriptions off'
        case 'no_version':
            return 'No version is ready to load yet'
        case 'awaiting_review':
            return 'Waiting on code review before it can load'
        case null:
            return item.available ? null : 'Not loadable right now'
    }
}

/** One line for where the owner's pack stands in the Workshop. */
export function ownedStatus(item: OwnedItemT): string {
    if (item.hidden) return 'Hidden'
    if (!item.canList) return 'No version provides Workshop content yet'
    if (item.compat === 'none') return 'Not offered in the Workshop'
    if (!item.version) return 'No ready version'
    if (!item.listable)
        return item.version.runsCode ? 'Waiting on code review' : 'Not loadable yet'

    return item.compat === 'any' ? 'Offered on every game' : `Offered on ${item.games.length} game(s)`
}

/** Toggle one id in a set of game ids, keeping their order stable. */
export function toggleId(ids: number[], id: number): number[] {
    return ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id]
}

/** Human size for a pack. */
export function packSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`

    return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}
