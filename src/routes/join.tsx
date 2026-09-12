import { useMemo } from 'react'
import { Link, useSearchParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { FiAlertTriangle, FiServer, FiUsers } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { GameIcon } from '~/components/game-icon'
import { LiveLatency } from '~/components/server-live'
import { requestFor, useLiveServer } from '~/lib/hooks/use-live-query'
import { appLabel } from '~/lib/api/labels'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'
import { useState } from 'react'
import type { ContentSummaryT } from '~/lib/api/contract'

/**
 * **Join** — where a `tmc://play/<host>:<port>` link lands.
 *
 * THE LINK SHOWS THIS PAGE. IT DOES NOT JOIN.
 * ------------------------------------------
 * That is the deep-link module's rule and it is load-bearing here rather than
 * theoretical, because this is the one link an ordinary web page can navigate
 * to without a click — any program on the machine can claim a custom scheme, so
 * a link is an untrusted request from an unknown party. A version that joined on
 * arrival would be a remote primitive for making somebody's machine connect to
 * an address a stranger chose. So the link resolves the address, shows what is
 * actually running there, and puts a button under it.
 *
 * WHY THE ADDRESS IS LOOKED UP AT ALL
 * ----------------------------------
 * A link carries an address and nothing else — that is what makes it something
 * somebody can paste into a message. An address is not enough to launch
 * anything: the app has to know WHICH GAME is there before it can decide
 * between a TMC build, a sandbox and the browser window, and it has to know the
 * server's id before it can ask Rust to resolve the connection. `/servers/lookup`
 * answers both, and discloses nothing — the caller already had the address.
 *
 * A HOST RUNNING TWO SERVERS IS A CHOICE, NOT A GUESS
 * --------------------------------------------------
 * A link with no port, or a box with several games on one address, comes back
 * with more than one row. They are all drawn. Picking one for somebody would be
 * picking which game they meant.
 */

export default function JoinRoute() {
    const [params] = useSearchParams()
    const [play, setPlay] = useState<PlayTargetT | null>(null)

    const host = params.get('host') ?? ''
    const portRaw = params.get('port')
    const port = portRaw ? Number(portRaw) : undefined

    const lookup = useQuery({
        queryKey: ['serverLookup', host, port ?? null],
        queryFn: () => api.serverLookup(host, port),
        enabled: host.length > 0,
        // A link is followed once. Re-asking on every focus would re-ask about
        // an address somebody has already dealt with.
        staleTime: 60 * 1000,
    })

    const address = useMemo(
        () => (port ? `${host}:${String(port)}` : host),
        [host, port]
    )

    if (!host)
        return (
            <Notice>
                That link did not name a server address.
                <Link to="/browse/server" className="ml-1 underline">
                    Browse servers
                </Link>
            </Notice>
        )

    const servers = lookup.data?.servers ?? []

    return (
        <div className="flex flex-col gap-3 p-3">
            <header className="flex flex-col gap-1">
                <h1 className="text-sm font-semibold">Join a server</h1>
                <p className="selectable text-xs text-muted">{address}</p>
            </header>

            {lookup.isPending && <Notice>Looking that address up&hellip;</Notice>}

            {lookup.isError && (
                <Notice danger>
                    Could not reach TMC to look that address up. Check your
                    connection and try the link again.
                </Notice>
            )}

            {lookup.isSuccess && servers.length === 0 && (
                <Notice>
                    No server TMC lists is at that address. It may be unlisted, or
                    the link may be out of date — you can still
                    <Link to="/browse/server" className="mx-1 underline">
                        browse servers
                    </Link>
                    to find it.
                </Notice>
            )}

            {servers.map((server) => (
                <ServerRow
                    key={server.id}
                    server={server}
                    onJoin={() =>
                        setPlay({
                            appId: server.app?.id ?? 0,
                            server,
                            fallback: server.app
                                ? {
                                      name: appLabel(server.app),
                                      slug: server.app.url ?? null,
                                      icon: server.app.icon ?? null,
                                  }
                                : undefined,
                        })
                    }
                />
            ))}

            {play && <PlayDialog target={play} onClose={() => setPlay(null)} />}
        </div>
    )
}

function Notice({
    children,
    danger,
}: {
    children: React.ReactNode
    danger?: boolean
}) {
    return (
        <div
            className={`rounded-xl border p-6 text-center text-sm ${
                danger
                    ? 'border-danger/40 bg-danger/10 text-danger'
                    : 'border-border text-muted'
            }`}
        >
            {danger && <FiAlertTriangle className="mx-auto mb-2 size-4" />}
            {children}
        </div>
    )
}

function ServerRow({
    server,
    onJoin,
}: {
    server: ContentSummaryT
    onJoin: () => void
}) {
    // Measured from THIS device, which is the one fact the website cannot give
    // somebody: a Sydney player and a Frankfurt player see different numbers
    // for the same box.
    const live = useLiveServer<HTMLDivElement>(requestFor(server))

    return (
        <article
            ref={live.ref}
            className="flex items-start gap-3 rounded-xl border border-border bg-surface p-3"
        >
            <GameIcon
                app={
                    server.app
                        ? {
                              id: server.app.id,
                              name: appLabel(server.app),
                              icon: server.app.icon,
                          }
                        : { id: 0, name: 'Server', icon: null }
                }
                size="lg"
            />

            <div className="min-w-0 flex-1">
                <h2 className="truncate text-sm font-semibold">{server.name}</h2>

                <p className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-muted">
                    {server.app && (
                        <span className="truncate">{appLabel(server.app)}</span>
                    )}

                    <span className="flex items-center gap-1">
                        <FiUsers className="size-3" />
                        {server.server?.curUsers ?? 0}
                        {server.server?.maxUsers
                            ? `/${String(server.server.maxUsers)}`
                            : ''}
                    </span>

                    <LiveLatency
                        result={live.result}
                        series={live.series}
                        error={live.error}
                        settled={live.settled}
                        probeable={!!server.server?.query}
                    />
                </p>

                <p className="mt-0.5 flex items-center gap-1 text-[11px] text-muted">
                    <FiServer className="size-3" />
                    <Link to={`/view/server/${server.id}`} className="underline">
                        Open the server&rsquo;s page
                    </Link>
                </p>
            </div>

            <button
                type="button"
                onClick={onJoin}
                disabled={!server.app}
                className="shrink-0 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
            >
                Join
            </button>
        </article>
    )
}
