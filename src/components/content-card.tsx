import type { Ref } from 'react'
import { Link } from 'react-router-dom'
import { FiDownload, FiExternalLink, FiEye, FiHeart, FiStar } from 'react-icons/fi'

import type { ContentSummaryT } from '~/lib/api/contract'
import { appLabel } from '~/lib/api/labels'
import { requestFor, useLiveServer } from '~/lib/hooks/use-live-query'
import { opensExternally, openExternally } from '~/lib/external'
import { useSettings } from '~/lib/settings/provider'
import { LiveLatency, LiveLatencyStrip, LivePlayers } from './server-live'

/**
 * One row in any browser.
 *
 * Deliberately the SAME component for every content kind. The API normalises
 * eight Prisma shapes into one `ContentSummary`, and the payoff for that is
 * here: a search across kinds, a favourites list and a game's page all render
 * with this and stay visually identical.
 *
 * Server rows get a live strip on top. The card registers itself with the
 * live-query registry while it is on screen (see `useLiveServer`), so scrolling
 * past a hundred servers queries the dozen the user can actually see rather
 * than all hundred.
 */

function compact(n: number): string {
    if (n < 1000) return String(n)
    if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`

    return `${(n / 1_000_000).toFixed(1)}m`
}

function Stat({
    icon: Icon,
    value,
    label,
}: {
    icon: typeof FiEye
    value: number
    label: string
}) {
    if (value <= 0) return null

    return (
        <span className="flex items-center gap-1" title={`${value} ${label}`}>
            <Icon className="size-3" />
            {compact(value)}
        </span>
    )
}

export default function ContentCard({ item }: { item: ContentSummaryT }) {
    const { app } = useSettings()
    const dense = app?.compactCards ?? false

    const request = requestFor(item)
    const live = useLiveServer<HTMLElement>(request)

    const image = item.images.card ?? item.images.banner ?? item.images.icon

    /*
     * The live probe wins over the API's cached flag once there is a verdict —
     * and a FAILED probe is a verdict. Falling back to the website's `online`
     * after our own query timed out would show a box that is demonstrably not
     * answering as "Online", which is the specific lie the live query exists to
     * prevent.
     */
    const online = live.error
        ? false
        : (live.result?.online ?? item.server?.online ?? false)
    const heading = live.result?.name ?? item.name

    /*
     * An article's card opens the browser rather than an in-app page — see
     * `lib/external`. It stays a card, and it keeps the live-query ref binding,
     * so nothing below this line has to know which of the two it is.
     */
    const external = opensExternally(item.kind)

    const shell =
        'group flex flex-col overflow-hidden rounded-xl border border-border bg-surface text-left transition-colors hover:border-accent'

    /*
     * The body is built ONCE and handed to whichever wrapper applies. Writing
     * the branch as an inline `<Frame>` component instead would give React a
     * new component type on every render, remounting the whole subtree — which
     * for this card means tearing down and rebuilding its IntersectionObserver,
     * and so re-registering with the live registry, several times a second.
     */
    const body = (
        <>
            <div
                className={`relative w-full shrink-0 overflow-hidden bg-surface-secondary ${
                    dense ? 'aspect-[3/1]' : 'aspect-[16/9]'
                }`}
            >
                {image ? (
                    <img
                        src={image}
                        alt=""
                        loading="lazy"
                        className="size-full object-cover transition-transform duration-200 group-hover:scale-105"
                    />
                ) : (
                    <div className="flex size-full items-center justify-center text-xs text-muted">
                        No preview
                    </div>
                )}

                {item.server && (
                    <span
                        className={`absolute left-2 top-2 rounded-full px-2 py-0.5 text-[0.65rem] font-medium ${
                            online
                                ? 'bg-success text-success-foreground'
                                : 'bg-danger text-danger-foreground'
                        }`}
                    >
                        {online ? 'Online' : 'Offline'}
                    </span>
                )}

                {item.isOfficial && (
                    <span className="absolute right-2 top-2 rounded-full bg-accent px-2 py-0.5 text-[0.65rem] font-medium text-accent-foreground">
                        Official
                    </span>
                )}
            </div>

            <div className="flex min-w-0 flex-1 flex-col gap-1.5 p-3">
                <div className="flex items-start justify-between gap-2">
                    <h3 className="min-w-0 flex-1 truncate text-sm font-semibold">
                        {heading}
                    </h3>

                    {external && (
                        <FiExternalLink
                            className="mt-0.5 size-3.5 shrink-0 text-muted"
                            aria-label="Opens in your browser"
                        />
                    )}

                    {item.server && live.enabled && (
                        <LiveLatency
                            result={live.result}
                            series={live.series}
                            error={live.error}
                            probeable={live.probeable}
                            settled={live.settled}
                        />
                    )}
                </div>

                {item.app && (
                    <p className="truncate text-xs text-accent">
                        {appLabel(item.app)}
                        {live.result?.map && (
                            <span className="text-muted"> · {live.result.map}</span>
                        )}
                    </p>
                )}

                {!dense && item.description && (
                    <p className="line-clamp-2 text-xs text-muted">
                        {item.description}
                    </p>
                )}

                <div className="mt-auto flex flex-wrap items-center gap-3 pt-1 text-[0.7rem] text-muted">
                    {item.server ? (
                        <LivePlayers item={item} result={live.result} />
                    ) : (
                        <>
                            <Stat
                                icon={FiDownload}
                                value={item.stats.downloads}
                                label="downloads"
                            />
                            <Stat
                                icon={FiEye}
                                value={item.stats.views}
                                label="views"
                            />
                            <Stat
                                icon={FiHeart}
                                value={item.stats.likes}
                                label="likes"
                            />
                        </>
                    )}

                    {item.stats.rating != null && (
                        <span className="flex items-center gap-1">
                            <FiStar className="size-3" />
                            {item.stats.rating.toFixed(1)}
                        </span>
                    )}

                    {item.archived && (
                        <span className="rounded bg-surface-tertiary px-1.5 py-0.5 text-[0.65rem]">
                            Archived
                        </span>
                    )}
                </div>

                {/* The history goes along the BOTTOM edge, under the stats
                    rather than beside the number. Sitting next to the reading
                    it made the row's most important value — the current ping —
                    compete with a sixty-point line for the same glance. */}
                {item.server && live.enabled && (
                    <LiveLatencyStrip series={live.series} />
                )}
            </div>
        </>
    )

    if (external)
        return (
            <button
                type="button"
                ref={live.ref as Ref<HTMLButtonElement>}
                onClick={() => void openExternally(item)}
                title="Opens in your browser"
                className={shell}
            >
                {body}
            </button>
        )

    return (
        <Link
            ref={live.ref as Ref<HTMLAnchorElement>}
            to={`/view/${item.kind}/${item.id}`}
            className={shell}
        >
            {body}
        </Link>
    )
}
