import { Link } from 'react-router-dom'
import { FiAlertTriangle, FiArrowRight, FiInfo } from 'react-icons/fi'

import type { ContentDependencyT } from '~/lib/api/contract'
import Markdown from '~/components/markdown'

/**
 * What an item needs, recommends, and cannot live beside.
 *
 * WHY CONFLICTS ARE NOT IN THE SAME LIST
 * -------------------------------------
 * Because they are the opposite instruction. Everything else here says "get
 * this too"; a conflict says "do not have both". Rendering them as one list
 * with a coloured badge is how somebody installs the thing that breaks their
 * game, so they get their own block, above, in the warning colour.
 *
 * The order within each block is the server's — required before advisory —
 * because it is the order somebody should read them in.
 */

const RELATION_LABEL: Record<ContentDependencyT['relation'], string> = {
    Required: 'Required',
    Recommended: 'Recommended',
    Optional: 'Optional',
    Conflict: 'Incompatible',
}

export default function Dependencies({ items }: { items: ContentDependencyT[] }) {
    if (items.length === 0) return null

    const conflicts = items.filter((item) => item.relation === 'Conflict')
    const needs = items.filter((item) => item.relation !== 'Conflict')

    return (
        <section className="flex flex-col gap-3">
            <h2 className="text-sm font-semibold">Works with</h2>

            {conflicts.length > 0 && (
                <div className="flex flex-col gap-1.5 rounded-xl border border-warning/50 bg-surface p-3">
                    <p className="flex items-center gap-1.5 text-xs font-medium text-warning">
                        <FiAlertTriangle className="size-3.5" />
                        Do not install alongside
                    </p>

                    {conflicts.map((item) => (
                        <Row key={`${item.kind}:${item.id}`} item={item} />
                    ))}
                </div>
            )}

            {needs.length > 0 && (
                <ul className="flex flex-col divide-y divide-border overflow-hidden rounded-xl border border-border bg-surface">
                    {needs.map((item) => (
                        <li key={`${item.kind}:${item.id}`} className="p-3">
                            <Row item={item} />
                        </li>
                    ))}
                </ul>
            )}
        </section>
    )
}

function Row({ item }: { item: ContentDependencyT }) {
    return (
        <div className="flex flex-col gap-1">
            <Link
                to={`/view/${item.kind}/${item.id}`}
                className="group flex items-center gap-2"
            >
                {item.icon ? (
                    <img
                        src={item.icon}
                        alt=""
                        loading="lazy"
                        className="size-7 shrink-0 rounded object-cover"
                    />
                ) : (
                    <div className="size-7 shrink-0 rounded bg-surface-tertiary" />
                )}

                <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm group-hover:text-accent">
                        {item.name}
                    </span>
                    <span
                        className={`block text-[0.7rem] ${
                            item.relation === 'Required'
                                ? 'text-accent'
                                : item.relation === 'Conflict'
                                  ? 'text-warning'
                                  : 'text-muted'
                        }`}
                    >
                        {RELATION_LABEL[item.relation]}
                    </span>
                </span>

                <FiArrowRight
                    aria-hidden
                    className="size-3.5 shrink-0 text-muted"
                />
            </Link>

            {item.note && (
                <div className="selectable flex gap-1.5 pl-9 text-[0.7rem] text-muted">
                    <FiInfo aria-hidden className="mt-0.5 size-3 shrink-0" />
                    {/* Author-written, so it goes through the sanitising
                        renderer like every other member-authored body. */}
                    <Markdown source={item.note} />
                </div>
            )}
        </div>
    )
}
