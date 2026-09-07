import { useState } from 'react'
import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query'
import { FiEdit2, FiStar, FiThumbsUp, FiTrash2 } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import type { ContentKindT, ReviewT } from '~/lib/api/contract'
import { useAuth } from '~/lib/auth/provider'
import Markdown from '~/components/markdown'
import Select from '~/components/select'

/**
 * What people said about an item.
 *
 * READ ONLY, and the screen says so rather than showing a disabled "write a
 * review" button. A button that cannot be pressed is worse than no button: it
 * reads as broken, while a line of text reads as a decision.
 *
 * TWO THINGS THIS DOES THAT A LIST WOULD NOT
 * ------------------------------------------
 *   * **The distribution, as bars.** An average of 4.1 made of forty fives and
 *     ten ones is a different item from one made of fifty fours, and only the
 *     bars tell them apart. The server computes the counts, because doing it
 *     here would mean fetching every review to draw five bars.
 *   * **Review bodies through the markdown renderer.** They are member-authored
 *     text, so they go through the same sanitising renderer a mod description
 *     does — never `dangerouslySetInnerHTML`, and never raw.
 *
 * WRITING ONE
 * -----------
 * Straight to the website's own `CreateReview`, through
 * `/reviews/write` — the same penalty check, the same owner-controlled reviews
 * toggle, the same configured text limits, the same activity feed row. One per
 * person per item, and a repeat call edits, because the model's unique
 * constraint makes a second insert an error rather than a second review.
 */

const SORTS = [
    { value: 'recent', label: 'Most recent' },
    { value: 'helpful', label: 'Most helpful' },
    { value: 'rating', label: 'Highest rated' },
] as const

type Sort = (typeof SORTS)[number]['value']

export default function Reviews({ kind, id }: { kind: ContentKindT; id: number }) {
    const [sort, setSort] = useState<Sort>('recent')
    const [writing, setWriting] = useState(false)

    const { status } = useAuth()
    const cache = useQueryClient()

    const refresh = () =>
        cache.invalidateQueries({ queryKey: ['reviews', kind, id] })

    const reviews = useInfiniteQuery({
        queryKey: ['reviews', kind, id, sort],
        queryFn: ({ pageParam }) =>
            api.reviews(kind, id, { sort, cursor: pageParam }),
        initialPageParam: null as string | null,
        getNextPageParam: (last) => last.nextCursor,
        staleTime: 60 * 1000,
    })

    const first = reviews.data?.pages[0]
    const rows = reviews.data?.pages.flatMap((page) => page.reviews) ?? []

    if (reviews.isPending)
        return (
            <section>
                <h2 className="mb-2 text-sm font-semibold">Reviews</h2>
                <p className="text-xs text-muted">Loading…</p>
            </section>
        )

    if (reviews.isError)
        return (
            <section>
                <h2 className="mb-2 text-sm font-semibold">Reviews</h2>
                <p className="text-xs text-muted">Reviews could not be loaded.</p>
            </section>
        )

    if (!first || first.total === 0)
        return (
            <section>
                <h2 className="mb-2 text-sm font-semibold">Reviews</h2>
                <p className="text-xs text-muted">Nobody has reviewed this yet.</p>
            </section>
        )

    const breakdown = [
        { score: 5, count: first.breakdown.five },
        { score: 4, count: first.breakdown.four },
        { score: 3, count: first.breakdown.three },
        { score: 2, count: first.breakdown.two },
        { score: 1, count: first.breakdown.one },
    ]

    const scored = breakdown.reduce((sum, row) => sum + row.count, 0)

    /*
     * The caller's own review, if it is on a page that has loaded. Used to
     * swap "write a review" for "edit yours" — showing the write button to
     * somebody who already has one produces a request that silently overwrites
     * what they wrote, which is the worst of both.
     */
    const mine = rows.find((review) => review.mine)

    return (
        <section className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <h2 className="text-sm font-semibold">
                    Reviews
                    <span className="ml-2 text-xs font-normal text-muted">
                        {first.total}
                    </span>
                </h2>

                <div className="flex items-center gap-2">
                    {status === 'signedIn' && !writing && !mine && (
                        <button
                            type="button"
                            onClick={() => setWriting(true)}
                            className="rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground"
                        >
                            Write a review
                        </button>
                    )}

                    <Select
                        label="Sort reviews"
                        value={sort}
                        onChange={setSort}
                        options={SORTS.map((s) => ({
                            value: s.value,
                            label: s.label,
                        }))}
                    />
                </div>
            </div>

            {writing && (
                <ReviewForm
                    kind={kind}
                    id={id}
                    existing={mine ?? null}
                    onDone={async () => {
                        setWriting(false)
                        await refresh()
                    }}
                    onCancel={() => setWriting(false)}
                />
            )}

            {first.average !== null && (
                <div className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3 sm:flex-row sm:items-center sm:gap-6">
                    <div className="shrink-0 text-center">
                        <p className="text-2xl font-bold">
                            {first.average.toFixed(1)}
                        </p>
                        <Stars value={first.average} />
                        <p className="mt-1 text-[0.7rem] text-muted">
                            {scored} rating{scored === 1 ? '' : 's'}
                        </p>
                    </div>

                    <div className="flex min-w-0 flex-1 flex-col gap-1">
                        {breakdown.map((row) => (
                            <div
                                key={row.score}
                                className="flex items-center gap-2 text-[0.7rem]"
                            >
                                <span className="w-3 text-right text-muted">
                                    {row.score}
                                </span>
                                <div className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-surface-tertiary">
                                    <div
                                        className="h-full rounded-full bg-accent"
                                        style={{
                                            width: `${
                                                scored === 0
                                                    ? 0
                                                    : (row.count / scored) * 100
                                            }%`,
                                        }}
                                    />
                                </div>
                                <span className="w-8 text-right text-muted">
                                    {row.count}
                                </span>
                            </div>
                        ))}
                    </div>
                </div>
            )}

            <ul className="flex flex-col gap-2">
                {rows.map((review) => (
                    <li
                        key={review.id}
                        className="rounded-xl border border-border bg-surface p-3"
                    >
                        <div className="flex items-center gap-2">
                            {review.owner?.avatar ? (
                                <img
                                    src={review.owner.avatar}
                                    alt=""
                                    loading="lazy"
                                    className="size-6 rounded-full object-cover"
                                />
                            ) : (
                                <div className="size-6 rounded-full bg-surface-tertiary" />
                            )}

                            <span className="min-w-0 truncate text-sm">
                                {review.owner?.username ??
                                    review.owner?.name ??
                                    'Someone'}
                            </span>

                            {review.rating !== null && (
                                <Stars value={review.rating} />
                            )}

                            <span className="ml-auto shrink-0 text-[0.7rem] text-muted">
                                {new Date(review.createdAt).toLocaleDateString()}
                                {review.lastEdit && ' · edited'}
                            </span>
                        </div>

                        {review.content && (
                            <div className="selectable mt-2 text-sm">
                                {/* Member-authored, so it goes through the same
                                    sanitising renderer a mod description does. */}
                                <Markdown source={review.content} />
                            </div>
                        )}

                        <div className="mt-2 flex items-center gap-3">
                            <HelpfulButton
                                review={review}
                                signedIn={status === 'signedIn'}
                                onVoted={refresh}
                            />

                            {review.mine && (
                                <>
                                    <button
                                        type="button"
                                        onClick={() => setWriting(true)}
                                        className="flex items-center gap-1 text-[0.7rem] text-muted hover:text-foreground"
                                    >
                                        <FiEdit2 className="size-3" />
                                        Edit
                                    </button>

                                    <button
                                        type="button"
                                        onClick={() => {
                                            void api
                                                .deleteReview(review.id)
                                                .then(refresh)
                                        }}
                                        className="flex items-center gap-1 text-[0.7rem] text-muted hover:text-danger"
                                    >
                                        <FiTrash2 className="size-3" />
                                        Delete
                                    </button>
                                </>
                            )}
                        </div>
                    </li>
                ))}
            </ul>

            {reviews.hasNextPage && (
                <button
                    type="button"
                    disabled={reviews.isFetchingNextPage}
                    onClick={() => void reviews.fetchNextPage()}
                    className="self-center rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent disabled:opacity-50"
                >
                    {reviews.isFetchingNextPage ? 'Loading…' : 'Show more'}
                </button>
            )}
        </section>
    )
}

/**
 * Leave or edit a review.
 *
 * The score is five buttons rather than a slider or a select: it is the one
 * input on this form people actually use, and a slider makes picking a 4 a
 * matter of aim.
 */
function ReviewForm({
    kind,
    id,
    existing,
    onDone,
    onCancel,
}: {
    kind: ContentKindT
    id: number
    existing: ReviewT | null
    onDone: () => Promise<unknown>
    onCancel: () => void
}) {
    const [rating, setRating] = useState<number | null>(existing?.rating ?? null)
    const [content, setContent] = useState(existing?.content ?? '')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const submit = async () => {
        setBusy(true)
        setError(null)

        try {
            await api.writeReview({
                kind,
                id,
                rating,
                content: content.trim() || null,
            })

            await onDone()
        } catch (err) {
            setError(
                err instanceof Error
                    ? err.message
                    : 'The review could not be saved.'
            )
        } finally {
            setBusy(false)
        }
    }

    // The server refuses a review that is neither, and matching that here means
    // the button is disabled rather than the request failing.
    const usable = rating !== null || content.trim().length > 0

    return (
        <form
            className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3"
            onSubmit={(e) => {
                e.preventDefault()
                void submit()
            }}
        >
            <div className="flex items-center gap-2">
                <span className="text-xs text-muted">Your score</span>

                <div className="flex">
                    {[1, 2, 3, 4, 5].map((score) => (
                        <button
                            key={score}
                            type="button"
                            aria-label={`${score} out of 5`}
                            aria-pressed={rating === score}
                            onClick={() =>
                                setRating(rating === score ? null : score)
                            }
                            className="p-0.5"
                        >
                            <FiStar
                                className={`size-4 ${
                                    rating !== null && score <= rating
                                        ? 'text-warning'
                                        : 'text-muted/40'
                                }`}
                                fill={
                                    rating !== null && score <= rating
                                        ? 'currentColor'
                                        : 'none'
                                }
                            />
                        </button>
                    ))}
                </div>

                {rating !== null && (
                    <button
                        type="button"
                        onClick={() => setRating(null)}
                        className="text-[0.7rem] text-muted hover:text-foreground"
                    >
                        Clear
                    </button>
                )}
            </div>

            <textarea
                value={content}
                rows={5}
                maxLength={20000}
                placeholder="What did you think? Markdown works."
                onChange={(e) => setContent(e.target.value)}
                className="w-full resize-y rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
            />

            {error && <p className="selectable text-xs text-danger">{error}</p>}

            <div className="flex gap-2">
                <button
                    type="submit"
                    disabled={busy || !usable}
                    className="rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground disabled:opacity-50"
                >
                    {busy ? 'Saving…' : existing ? 'Save changes' : 'Post'}
                </button>
                <button
                    type="button"
                    onClick={onCancel}
                    className="rounded-lg border border-border px-3 py-1.5 text-sm"
                >
                    Cancel
                </button>
            </div>
        </form>
    )
}

/**
 * The helpful count, and the button that changes it.
 *
 * Pressing it again takes the vote back — a helpful button with no way to
 * un-press it is one people press by accident once and resent forever. The
 * count moves optimistically because the server's answer is a foregone
 * conclusion and a number that lags a click by 300ms reads as a lost one.
 */
function HelpfulButton({
    review,
    signedIn,
    onVoted,
}: {
    review: ReviewT
    signedIn: boolean
    onVoted: () => Promise<unknown>
}) {
    const [vote, setVote] = useState(review.myVote)
    const [count, setCount] = useState(review.score)

    if (!signedIn || review.mine)
        return (
            <span className="flex items-center gap-1 text-[0.7rem] text-muted">
                <FiThumbsUp className="size-3" />
                {count}
            </span>
        )

    const toggle = () => {
        const next = vote === true ? null : true

        setVote(next)
        setCount((was) => was + (next === true ? 1 : -1))

        void api
            .voteReview(review.id, next)
            .then(onVoted)
            .catch(() => {
                // Put it back. A count that stayed wrong after a failed request
                // is one somebody reports as a bug in the counting.
                setVote(review.myVote)
                setCount(review.score)
            })
    }

    return (
        <button
            type="button"
            onClick={toggle}
            aria-pressed={vote === true}
            className={`flex items-center gap-1 text-[0.7rem] ${
                vote === true ? 'text-accent' : 'text-muted hover:text-foreground'
            }`}
        >
            <FiThumbsUp
                className="size-3"
                fill={vote === true ? 'currentColor' : 'none'}
            />
            {count}
            <span className="sr-only">helpful</span>
        </button>
    )
}

/**
 * Five stars, filled to `value`.
 *
 * The fill is a clipped overlay rather than half-star glyphs: a 4.3 is drawn as
 * 4.3, and there is no rounding decision to get wrong or to disagree with the
 * number printed beside it.
 */
function Stars({ value }: { value: number }) {
    const percent = Math.max(0, Math.min(100, (value / 5) * 100))

    return (
        <span
            className="relative inline-flex shrink-0"
            role="img"
            aria-label={`${value.toFixed(1)} out of 5`}
        >
            <span className="flex text-muted/40">
                {[0, 1, 2, 3, 4].map((n) => (
                    <FiStar key={n} className="size-3.5" aria-hidden />
                ))}
            </span>

            <span
                className="absolute inset-0 flex overflow-hidden text-warning"
                style={{ width: `${percent}%` }}
                aria-hidden
            >
                {[0, 1, 2, 3, 4].map((n) => (
                    <FiStar
                        key={n}
                        className="size-3.5 shrink-0"
                        fill="currentColor"
                    />
                ))}
            </span>
        </span>
    )
}
