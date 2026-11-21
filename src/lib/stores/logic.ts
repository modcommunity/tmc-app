import type { StoreT } from '~/lib/ipc/schemas'

/** Mirrors `MAX_NAME_CHARS` in `tmc_core::library::stores`. */
export const MAX_STORE_NAME = 64

/**
 * The reason a store name is unusable, or null. Rust is the authority (it also
 * checks uniqueness against the saved list); this only spares a round trip.
 */
export function storeNameProblem(
    raw: string,
    others: readonly string[] = []
): string | null {
    const name = raw.trim()

    if (name === '') return 'Give this store a name.'
    if ([...name].length > MAX_STORE_NAME) {
        return `A store name can be at most ${MAX_STORE_NAME} characters.`
    }

    const lower = name.toLowerCase()

    if (others.some((other) => other.trim().toLowerCase() === lower)) {
        return 'A store with that name already exists.'
    }

    return null
}

/**
 * Share of the volume this store's files take: used / (used + free). Null when
 * free space is unknown or there is nothing to draw.
 */
export function usageFraction(
    store: Pick<StoreT, 'usedBytes' | 'freeBytes'>
): number | null {
    if (store.freeBytes === null) return null

    const total = store.usedBytes + store.freeBytes

    if (total <= 0) return null

    return Math.min(1, Math.max(0, store.usedBytes / total))
}

/** Whether a store can be made the default (Rust enforces it too). */
export function canBeDefault(
    store: Pick<StoreT, 'kind' | 'isDefault' | 'exists'>
): boolean {
    return !store.isDefault && store.exists && store.kind === 'permanent'
}
