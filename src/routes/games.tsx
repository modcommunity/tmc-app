import { useEffect, useMemo, useState } from 'react'
import { useInfiniteQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { FiAlertTriangle, FiSearch, FiServer, FiUser, FiUsers } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { messageOf } from '~/lib/ipc'
import { compact, pageQuery, DEFAULT_FILTERS } from '~/lib/api/play'
import { canonicalSlug, gameHeadingStyle, gameLook, isPlatformSlug } from '~/lib/gamespice/games'
import GameServers from '~/components/game-servers'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'
import { BTN_GHOST, BTN_PRIMARY, type PlayAppRowT } from '~/components/play-center'

/**
 * **Games** — the GameSpice catalogue: the platform's own games, each in its
 * own colours and display face.
 *
 * Same endpoint as the Play center's Games tab (`GET /play/page`, which carries
 * the play vocabulary and per-game counts and answers for a signed-out visitor,
 * so nothing here is behind sign-in), narrowed to the platform allowlist in
 * `lib/gamespice/games.ts`. Starting a game goes through `PlayDialog`, the one
 * launcher; this screen decides nothing about whether a launch works.
 */
export default function GamesRoute() {
    const [q, setQ] = useState('')
    const [target, setTarget] = useState<PlayTargetT | null>(null)

    const listing = useInfiniteQuery({
        queryKey: ['gamespice', 'games'],
        initialPageParam: 0,
        queryFn: ({ pageParam }) =>
            api.playPage(pageQuery({ ...DEFAULT_FILTERS, tab: 'games' }, pageParam)),
        getNextPageParam: (last) => last?.nextCursor ?? undefined,
        staleTime: 60_000,
    })

    // No cap: read every page, the platform's games are a short list inside a long one.
    const { hasNextPage, isFetchingNextPage, fetchNextPage } = listing

    useEffect(() => {
        if (hasNextPage && !isFetchingNextPage) void fetchNextPage()
    }, [hasNextPage, isFetchingNextPage, fetchNextPage])

    const games = useMemo(
        () =>
            (listing.data?.pages.flatMap((p) => p?.apps ?? []) ?? [])
                .filter((a) => isPlatformSlug(a.url))
                .sort((a, b) => b.playerCnt - a.playerCnt || a.name.localeCompare(b.name)),
        [listing.data]
    )

    const shown = useMemo(() => {
        const needle = q.trim().toLowerCase()

        return needle ? games.filter((g) => g.name.toLowerCase().includes(needle)) : games
    }, [games, q])

    const loading = listing.isPending || hasNextPage

    return (
        <div className="flex flex-col gap-4 p-4">
            <header className="flex flex-wrap items-end justify-between gap-3">
                <div>
                    <h1 className="text-lg font-bold">GameSpice games</h1>
                    <p className="text-xs text-muted">
                        Free games built on the GameSpice platform. Play in a window, or join a
                        live server.
                    </p>
                </div>

                <label className="flex items-center gap-2 rounded-lg border border-border px-2.5 py-1.5 text-xs">
                    <FiSearch className="size-3.5 text-muted" />
                    <input
                        value={q}
                        onChange={(e) => setQ(e.target.value)}
                        placeholder="Search games"
                        aria-label="Search games"
                        className="w-44 bg-transparent outline-none"
                    />
                </label>
            </header>

            {listing.isError && (
                <p role="alert" className="flex items-center gap-2 text-sm text-danger">
                    <FiAlertTriangle className="size-4" />
                    {messageOf(listing.error)}
                </p>
            )}

            {listing.data?.pages[0] === null && (
                <p className="text-sm text-muted">Play is switched off on this site.</p>
            )}

            {loading && <p className="text-sm text-muted">Loading games…</p>}

            {!loading && !listing.isError && shown.length === 0 && (
                <p className="text-sm text-muted">
                    {games.length === 0
                        ? 'No GameSpice games are listed on this site yet.'
                        : 'No game matches that search.'}
                </p>
            )}

            <ul className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
                {shown.map((game) => (
                    <li key={game.id}>
                        <PlatformGameCard
                            game={game}
                            onPlay={() =>
                                setTarget({
                                    appId: game.id,
                                    fallback: {
                                        name: game.name,
                                        icon: game.iconUrl,
                                        slug: game.url && canonicalSlug(game.url),
                                    },
                                })
                            }
                        />
                    </li>
                ))}
            </ul>

            {!loading && shown.length > 0 && (
                <GameServers appIds={shown.slice(0, 50).map((g) => g.id)} limit={15} />
            )}

            {target && <PlayDialog target={target} onClose={() => setTarget(null)} />}
        </div>
    )
}

function PlatformGameCard({ game, onPlay }: { game: PlayAppRowT; onPlay: () => void }) {
    const look = gameLook(game.url ?? game.name)
    const art = game.bannerUrl ?? game.iconUrl
    const slug = game.url ? canonicalSlug(game.url) : String(game.id)

    return (
        <article
            className="flex h-full flex-col overflow-hidden rounded-2xl border border-border bg-surface"
            style={{ borderTopColor: look.accent, borderTopWidth: 3 }}
        >
            <div className="relative aspect-[16/8] w-full overflow-hidden bg-surface-2">
                {art ? (
                    <img src={art} alt="" aria-hidden loading="lazy" className="size-full object-cover" />
                ) : (
                    <div
                        className="size-full"
                        style={{
                            background: `linear-gradient(135deg, hsl(${look.hue} 60% 35%), hsl(${(look.hue + 40) % 360} 50% 18%))`,
                        }}
                    />
                )}
                <div className="absolute inset-0 bg-gradient-to-t from-surface via-transparent to-transparent" />
            </div>

            <div className="flex flex-1 flex-col gap-3 p-4">
                <div>
                    <h2 className="text-xl font-extrabold leading-tight" style={gameHeadingStyle(look)}>
                        {game.name}
                    </h2>
                    {game.description && (
                        <p className="mt-1 line-clamp-2 text-xs text-muted">{game.description}</p>
                    )}
                </div>

                <p className="flex flex-wrap items-center gap-3 text-[11px] text-muted">
                    <span className="flex items-center gap-1">
                        <FiUsers className="size-3" />
                        {compact(game.playerCnt)} playing
                    </span>
                    <span className="flex items-center gap-1">
                        <FiServer className="size-3" />
                        {compact(game.serverCnt)} servers
                    </span>
                    {game.solo && (
                        <span className="flex items-center gap-1">
                            <FiUser className="size-3" />
                            Solo
                        </span>
                    )}
                </p>

                <div className="mt-auto flex flex-wrap gap-2">
                    <button type="button" onClick={onPlay} className={BTN_PRIMARY}>
                        Play
                    </button>
                    <Link to={`/games/${slug}`} className={BTN_GHOST}>
                        Open
                    </Link>
                    <Link to={`/games/${slug}/avatar`} className={BTN_GHOST}>
                        Avatar
                    </Link>
                </div>
            </div>
        </article>
    )
}
