/**
 * An item's own artwork, in a list of items.
 *
 * The sandbox mod list and the download queue are the two screens where a user
 * is looking at ten or forty rows that all read `Cool Mod`, `Cooler Mod`,
 * `Coolest Mod` — a wall of similar text where the thing they are looking for
 * is the one they recognise by its cover.
 *
 * The image comes from the LOCAL library row, not from a request. Every row in
 * these lists is something the user subscribed to, so its card image is already
 * in `subscription` on this device — `LibraryRow.image` — and a lookup is a map
 * hit rather than forty network calls in a scrolling list.
 *
 * Falls back to a neutral tile with the kind's initial rather than to nothing,
 * for the same reason [`GameIcon`] does: rows where some are indented by an
 * image and some are not read as broken.
 */

import { FiBox, FiHardDrive, FiImage, FiLayers } from 'react-icons/fi'

const SIZES = {
    sm: 'h-6 w-6 rounded text-[10px]',
    md: 'h-9 w-9 rounded-md text-xs',
} as const

export function ItemThumb({
    image,
    kind,
    size = 'md',
    className = '',
}: {
    image: string | null | undefined
    /**
     * `mod`, `asset`, `collection` or `local` — decides the placeholder glyph.
     *
     * `local` is an imported mod, which never has an image: there is no item
     * page behind it and nothing to fetch a cover from. Its own glyph is what
     * stops a load order full of imports looking like a load order full of
     * subscriptions whose artwork failed to load.
     */
    kind?: string | null
    size?: keyof typeof SIZES
    className?: string
}) {
    const box = `${SIZES[size]} shrink-0 overflow-hidden border border-border ${className}`

    if (image) {
        return (
            <img
                src={image}
                alt=""
                loading="lazy"
                aria-hidden
                className={`${box} object-cover`}
            />
        )
    }

    const Glyph =
        kind === 'collection'
            ? FiLayers
            : kind === 'asset'
              ? FiImage
              : kind === 'local'
                ? FiHardDrive
                : FiBox

    return (
        <span
            aria-hidden
            className={`${box} flex items-center justify-center bg-surface-2 text-muted`}
        >
            <Glyph className="size-3.5" />
        </span>
    )
}
