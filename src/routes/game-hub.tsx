import { useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { FiArrowLeft, FiServer, FiUser, FiUsers } from 'react-icons/fi'

import { messageOf } from '~/lib/ipc'
import { compact } from '~/lib/api/play'
import { canonicalSlug, gameHeadingStyle, gameLook, isPlatformSlug } from '~/lib/gamespice/games'
import { useGameBySlug } from '~/lib/gamespice/use-game'
import GameServers from '~/components/game-servers'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'
import { BTN_GHOST, BTN_PRIMARY } from '~/components/play-center'

/**
 * **Game hub** — one GameSpice game: its art and face, play, solo, its live
 * servers (the Play center, narrowed to this game) and the avatar entry point.
 * The avatar editor itself belongs to `/games/:slug/avatar`.
 */
export default function GameHubRoute() {
    const { slug: raw } = useParams()
    const slug = raw ? canonicalSlug(raw) : undefined
    const game = useGameBySlug(slug)
    const [target, setTarget] = useState<PlayTargetT | null>(null)

    if (!slug || !isPlatformSlug(slug))
        return (
            <div className="flex flex-col gap-3 p-4">
                <p className="text-sm text-muted">That is not a GameSpice game.</p>
                <Link to="/games" className={BTN_GHOST}>
                    All games
                </Link>
            </div>
        )

    const row = game.data
    const look = gameLook(slug)
    const art = row?.bannerUrl ?? row?.iconUrl

    return (
        <div className="flex flex-col gap-4 p-4">
            <Link to="/games" className="flex items-center gap-1 text-xs text-muted">
                <FiArrowLeft className="size-3" /> GameSpice games
            </Link>

            {game.isError && (
                <p role="alert" className="text-sm text-danger">
                    {messageOf(game.error)}
                </p>
            )}
            {game.isPending && <p className="text-sm text-muted">Loading…</p>}
            {game.isSuccess && !row && (
                <p className="text-sm text-muted">This game is not listed on this site right now.</p>
            )}

            {row && (
                <>
                    <div
                        className="relative overflow-hidden rounded-2xl border border-border"
                        style={{ borderTopColor: look.accent, borderTopWidth: 3 }}
                    >
                        {art && (
                            <img src={art} alt="" aria-hidden className="h-48 w-full object-cover" />
                        )}
                        <div className="flex flex-col gap-3 p-5">
                            <h1 className="text-3xl font-extrabold" style={gameHeadingStyle(look)}>
                                {row.name}
                            </h1>
                            {row.description && <p className="text-sm text-muted">{row.description}</p>}
                            <p className="flex flex-wrap gap-4 text-xs text-muted">
                                <span className="flex items-center gap-1">
                                    <FiUsers className="size-3.5" /> {compact(row.playerCnt)} playing
                                </span>
                                <span className="flex items-center gap-1">
                                    <FiServer className="size-3.5" /> {compact(row.serverCnt)} servers
                                </span>
                                {row.solo && (
                                    <span className="flex items-center gap-1">
                                        <FiUser className="size-3.5" /> Solo play
                                    </span>
                                )}
                            </p>
                            <div className="flex flex-wrap gap-2">
                                <button
                                    type="button"
                                    className={BTN_PRIMARY}
                                    onClick={() =>
                                        setTarget({
                                            appId: row.id,
                                            fallback: { name: row.name, icon: row.iconUrl, slug },
                                        })
                                    }
                                >
                                    Play
                                </button>
                                <Link to={`/play?app=${row.id}&tab=servers`} className={BTN_GHOST}>
                                    Servers and lobbies
                                </Link>
                                <Link to={`/games/${slug}/avatar`} className={BTN_GHOST}>
                                    Avatar
                                </Link>
                                <Link to={`/apps/${row.id}`} className={BTN_GHOST}>
                                    Mods and packs
                                </Link>
                            </div>
                        </div>
                    </div>

                    <GameServers appIds={[row.id]} />
                </>
            )}

            {target && <PlayDialog target={target} onClose={() => setTarget(null)} />}
        </div>
    )
}
