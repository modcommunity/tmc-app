import { useState, type ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { FiBox, FiCode, FiDownload, FiStar, FiUsers } from 'react-icons/fi'

import { packSize } from '~/lib/gamespice/workshop'
import {
    compactCount,
    filledStars,
    formatRating,
    initials,
    kindGradient,
    relativeDate,
} from '~/lib/gamespice/workshop-format'

export type PackCardProps = {
    name: string
    href: string
    image: string | null
    summary?: string | null
    author?: { name: string; avatar?: string | null } | null
    official?: boolean
    kinds?: string[]
    /** "Every game", a list of names, or nothing. */
    games?: string
    version?: string | null
    bytes?: number | null
    subscribers?: number | null
    downloads?: number | null
    rating?: number | null
    ratingCount?: number | null
    runsCode?: boolean
    updatedAt?: string | null
    /** Small status pill on the banner (Subscribed, Paused, ...). */
    status?: string | null
    /** The state-aware action, or controls. */
    footer?: ReactNode
    testId?: string
}

const chip =
    'inline-flex items-center gap-1 rounded-full bg-surface-2 px-2 py-0.5 text-[11px] text-muted'
const overlay =
    'rounded-full bg-black/60 px-2 py-0.5 text-[11px] font-medium text-white backdrop-blur-sm'

export function PackCardSkeleton() {
    return (
        <li
            aria-hidden="true"
            className="overflow-hidden rounded-xl border border-border bg-surface motion-safe:animate-pulse"
        >
            <div className="aspect-video w-full bg-surface-2" />
            <div className="flex flex-col gap-2 p-3">
                <div className="h-4 w-2/3 rounded bg-surface-2" />
                <div className="h-3 w-full rounded bg-surface-2" />
                <div className="h-3 w-4/5 rounded bg-surface-2" />
                <div className="h-8 w-full rounded bg-surface-2" />
            </div>
        </li>
    )
}

export function PackCard(p: PackCardProps) {
    const [broken, setBroken] = useState(false)
    const [avatarBroken, setAvatarBroken] = useState(false)
    const rating = formatRating(p.rating)
    const stars = filledStars(p.rating)
    const updated = relativeDate(p.updatedAt)
    const showImage = p.image && !broken

    return (
        <li
            data-testid={p.testId}
            className="group flex flex-col overflow-hidden rounded-xl border border-border bg-surface transition duration-200 motion-safe:hover:-translate-y-0.5 hover:border-accent/60 hover:shadow-lg focus-within:border-accent/60 motion-reduce:transition-none"
        >
            <Link
                to={p.href}
                aria-label={`${p.name}, open pack page`}
                className="relative block aspect-video w-full overflow-hidden bg-surface-2 outline-none focus-visible:ring-2 focus-visible:ring-accent"
                style={
                    showImage
                        ? undefined
                        : { backgroundImage: kindGradient(p.kinds?.[0]) }
                }
            >
                {showImage ? (
                    <img
                        src={p.image!}
                        alt=""
                        loading="lazy"
                        decoding="async"
                        onError={() => setBroken(true)}
                        className="size-full object-cover transition-transform duration-300 motion-safe:group-hover:scale-105 motion-reduce:transition-none"
                    />
                ) : (
                    <span className="absolute inset-0 flex items-center justify-center text-white/40">
                        <FiBox className="size-10" aria-hidden="true" />
                    </span>
                )}
                <span className="absolute left-2 top-2 flex flex-wrap gap-1">
                    {p.kinds?.slice(0, 2).map((k) => (
                        <span key={k} className={overlay}>
                            {k}
                        </span>
                    ))}
                </span>
                <span className="absolute bottom-2 left-2 flex flex-wrap gap-1">
                    {p.games && <span className={overlay}>{p.games}</span>}
                </span>
                <span className="absolute right-2 top-2 flex flex-col items-end gap-1">
                    {p.status && (
                        <span className="rounded-full bg-accent px-2 py-0.5 text-[11px] font-semibold text-white">
                            {p.status}
                        </span>
                    )}
                    {p.runsCode && (
                        <span
                            className={`${overlay} inline-flex items-center gap-1`}
                            title="This pack runs code"
                        >
                            <FiCode aria-hidden="true" /> runs code
                        </span>
                    )}
                </span>
            </Link>
            <div className="flex flex-1 flex-col gap-2 p-3">
                <div className="flex items-start justify-between gap-2">
                    <Link
                        to={p.href}
                        title={p.name}
                        className="line-clamp-1 min-w-0 break-words text-sm font-semibold hover:underline"
                    >
                        {p.name}
                    </Link>
                    {p.version && <span className={chip}>v{p.version}</span>}
                </div>
                {p.author && (
                    <p className="flex items-center gap-1.5 text-xs text-muted">
                        {p.author.avatar && !avatarBroken ? (
                            <img
                                src={p.author.avatar}
                                alt=""
                                loading="lazy"
                                onError={() => setAvatarBroken(true)}
                                className="size-5 rounded-full object-cover"
                            />
                        ) : (
                            <span
                                aria-hidden="true"
                                className="flex size-5 items-center justify-center rounded-full bg-surface-2 text-[9px] font-bold"
                            >
                                {initials(p.author.name)}
                            </span>
                        )}
                        <span className="truncate">by {p.author.name}</span>
                        {p.official && (
                            <span className="rounded bg-accent/20 px-1 text-[10px] text-accent">
                                official
                            </span>
                        )}
                    </p>
                )}
                {p.summary && (
                    <p className="line-clamp-2 text-xs text-muted">{p.summary}</p>
                )}
                <div className="flex flex-wrap items-center gap-1.5">
                    {rating && (
                        <span
                            className={chip}
                            aria-label={`Rated ${rating} out of 5${p.ratingCount ? ` by ${p.ratingCount}` : ''}`}
                        >
                            <span className="inline-flex text-amber-400">
                                {Array.from({ length: 5 }, (_, i) => (
                                    <FiStar
                                        key={i}
                                        className={i < stars ? 'fill-current' : ''}
                                        aria-hidden="true"
                                    />
                                ))}
                            </span>
                            {rating}
                            {p.ratingCount
                                ? ` (${compactCount(p.ratingCount)})`
                                : ''}
                        </span>
                    )}
                    {p.subscribers != null && (
                        <span className={chip} title="Subscribers">
                            <FiUsers aria-hidden="true" />{' '}
                            {compactCount(p.subscribers)}
                            <span className="sr-only"> subscribers</span>
                        </span>
                    )}
                    {p.downloads != null && (
                        <span className={chip} title="Downloads">
                            <FiDownload aria-hidden="true" />{' '}
                            {compactCount(p.downloads)}
                            <span className="sr-only"> downloads</span>
                        </span>
                    )}
                    {p.bytes != null && p.bytes > 0 && (
                        <span className={chip}>{packSize(p.bytes)}</span>
                    )}
                    {updated && <span className={chip}>updated {updated}</span>}
                </div>
                {p.footer && <div className="mt-auto pt-1">{p.footer}</div>}
            </div>
        </li>
    )
}
