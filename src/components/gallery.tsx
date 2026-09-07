import { useEffect, useState } from 'react'
import { FiChevronLeft, FiChevronRight, FiX } from 'react-icons/fi'

/**
 * A screenshot strip, and the full-size view behind it.
 *
 * The website shows these at the width of a content column; an app window is
 * often narrower than one screenshot, so the strip is thumbnails and the real
 * image is a layer over the whole window. That is the difference between "there
 * are screenshots" and "you can see the screenshots", and it is most of why
 * this is a component rather than four lines of `<img>` in the view route.
 *
 * KEYBOARD, BECAUSE THIS IS A DESKTOP APP TOO
 * -------------------------------------------
 * Arrows move, Escape closes. A lightbox that can only be driven by clicking is
 * one that stops working the moment somebody is holding a controller-shaped
 * mouse in one hand and a drink in the other, which is the actual posture of
 * somebody browsing mods.
 */

export type GalleryItem = {
    id: string
    url: string
    title: string | null
}

export default function Gallery({ items }: { items: GalleryItem[] }) {
    const [openAt, setOpenAt] = useState<number | null>(null)

    if (items.length === 0) return null

    return (
        <>
            <div className="flex gap-2 overflow-x-auto pb-1">
                {items.map((item, index) => (
                    <button
                        key={item.id}
                        type="button"
                        onClick={() => setOpenAt(index)}
                        className="shrink-0 overflow-hidden rounded-lg border border-transparent transition-colors hover:border-accent"
                        aria-label={item.title ?? `Screenshot ${index + 1}`}
                    >
                        <img
                            src={item.url}
                            alt={item.title ?? ''}
                            loading="lazy"
                            className="h-32 w-auto object-cover"
                        />
                    </button>
                ))}
            </div>

            {openAt !== null && (
                <Lightbox
                    items={items}
                    at={openAt}
                    onMove={setOpenAt}
                    onClose={() => setOpenAt(null)}
                />
            )}
        </>
    )
}

function Lightbox({
    items,
    at,
    onMove,
    onClose,
}: {
    items: GalleryItem[]
    at: number
    onMove: (next: number) => void
    onClose: () => void
}) {
    const current = items[at]

    useEffect(() => {
        const onKey = (event: KeyboardEvent) => {
            if (event.key === 'Escape') onClose()
            if (event.key === 'ArrowRight') onMove((at + 1) % items.length)
            if (event.key === 'ArrowLeft')
                onMove((at - 1 + items.length) % items.length)
        }

        window.addEventListener('keydown', onKey)

        return () => window.removeEventListener('keydown', onKey)
    }, [at, items.length, onMove, onClose])

    if (!current) return null

    return (
        <div
            className="fixed inset-0 z-50 flex flex-col bg-black/90"
            role="dialog"
            aria-modal="true"
            aria-label={current.title ?? 'Screenshot'}
            onClick={(e) => e.target === e.currentTarget && onClose()}
        >
            <div
                className="flex items-center justify-between gap-3 p-3"
                style={{ paddingTop: 'calc(0.75rem + var(--safe-top))' }}
            >
                <p className="min-w-0 truncate text-sm text-white">
                    {current.title ?? `${at + 1} of ${items.length}`}
                </p>

                <button
                    type="button"
                    onClick={onClose}
                    aria-label="Close"
                    className="rounded-full bg-white/10 p-2 text-white"
                >
                    <FiX className="size-4" />
                </button>
            </div>

            <div className="flex min-h-0 flex-1 items-center gap-2 px-2">
                {items.length > 1 && (
                    <button
                        type="button"
                        aria-label="Previous"
                        onClick={() =>
                            onMove((at - 1 + items.length) % items.length)
                        }
                        className="shrink-0 rounded-full bg-white/10 p-2 text-white"
                    >
                        <FiChevronLeft className="size-5" />
                    </button>
                )}

                <img
                    src={current.url}
                    alt={current.title ?? ''}
                    className="mx-auto max-h-full min-w-0 flex-1 object-contain"
                />

                {items.length > 1 && (
                    <button
                        type="button"
                        aria-label="Next"
                        onClick={() => onMove((at + 1) % items.length)}
                        className="shrink-0 rounded-full bg-white/10 p-2 text-white"
                    >
                        <FiChevronRight className="size-5" />
                    </button>
                )}
            </div>

            {items.length > 1 && (
                <div
                    className="flex gap-1.5 overflow-x-auto p-3"
                    style={{ paddingBottom: 'calc(0.75rem + var(--safe-bottom))' }}
                >
                    {items.map((item, index) => (
                        <button
                            key={item.id}
                            type="button"
                            onClick={() => onMove(index)}
                            aria-label={`Show ${item.title ?? `screenshot ${index + 1}`}`}
                            className={`shrink-0 overflow-hidden rounded border-2 ${
                                index === at
                                    ? 'border-accent'
                                    : 'border-transparent opacity-60'
                            }`}
                        >
                            <img
                                src={item.url}
                                alt=""
                                loading="lazy"
                                className="h-12 w-auto object-cover"
                            />
                        </button>
                    ))}
                </div>
            )}
        </div>
    )
}
