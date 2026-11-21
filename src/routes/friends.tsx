import { useState } from 'react'
import { SkeletonGrid } from '~/components/skeleton'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import {
    FiCheck,
    FiLogOut,
    FiRefreshCw,
    FiServer,
    FiUserMinus,
    FiUsers,
    FiX,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import {
    presenceLabel,
    type FriendRequestT,
    type FriendT,
    type PartyT,
    type PresenceT,
} from '~/lib/api/social'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'

/**
 * **Friends** — who you play with, where they are, and the party you are in.
 *
 * The same friend list, requests and presence the website's friend dock and a
 * game's own `dot-friends` panel read, over the same routes: a request accepted
 * here is accepted in the game, and the other way round.
 *
 * WHAT THIS SCREEN DOES NOT DO
 * ---------------------------
 * Post presence. Only a GAME token places its player (a device token's post
 * keeps `status` and drops the server and party — docs/api/app-social.md), so
 * "in a game" is the game's to say. It also does not follow a friend INTO a
 * server by itself: "Join" opens the server's page, whose Play button is the
 * one launcher, rather than a second place a launch is decided.
 */

/** Presence goes stale after 120s server-side; a minute keeps the dots honest. */
const POLL_MS = 60_000

const STATUS_TONE: Record<PresenceT['status'], string> = {
    in_game: 'bg-success',
    online: 'bg-accent',
    away: 'bg-warning',
    offline: 'bg-border',
}

export default function FriendsRoute() {
    const { status } = useAuth()

    if (status !== 'signedIn')
        return (
            <div className="flex flex-col gap-3 p-3">
                <h1 className="text-lg font-bold">Friends</h1>
                <div className="rounded-xl border border-border p-6 text-center">
                    <p className="text-sm text-muted">
                        Sign in to see your friends, their games and your party.
                    </p>
                    <Link
                        to="/account"
                        className="mt-3 inline-block rounded-lg bg-accent px-4 py-2 text-sm text-accent-foreground"
                    >
                        Sign in
                    </Link>
                </div>
            </div>
        )

    return <SignedIn />
}

function SignedIn() {
    const client = useQueryClient()

    const friends = useQuery({
        queryKey: ['social', 'friends'],
        queryFn: api.friends,
        refetchInterval: POLL_MS,
    })
    const requests = useQuery({
        queryKey: ['social', 'requests'],
        queryFn: api.friendRequests,
        refetchInterval: POLL_MS,
    })
    const party = useQuery({
        queryKey: ['social', 'party'],
        queryFn: api.partyMine,
        refetchInterval: POLL_MS,
    })
    const invites = useQuery({
        queryKey: ['social', 'invites'],
        queryFn: api.partyInvites,
        refetchInterval: POLL_MS,
    })

    const refresh = () => void client.invalidateQueries({ queryKey: ['social'] })

    const incoming = requests.data?.incoming ?? []
    const outgoing = requests.data?.outgoing ?? []
    const list = [...(friends.data ?? [])].sort(byPresence)

    return (
        <div className="flex flex-col gap-4 p-3">
            <div className="flex items-center justify-between gap-2">
                <h1 className="flex items-center gap-2 text-lg font-bold">
                    <FiUsers className="size-4" />
                    Friends
                    {list.length > 0 && (
                        <span className="text-sm font-normal text-muted">
                            {
                                list.filter((f) => f.presence.status !== 'offline')
                                    .length
                            }{' '}
                            online of {list.length}
                        </span>
                    )}
                </h1>
                <button
                    type="button"
                    onClick={refresh}
                    className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-xs hover:border-accent"
                >
                    <FiRefreshCw className="size-3" />
                    Refresh
                </button>
            </div>

            <PartySection party={party.data ?? null} error={party.error} />

            {/* Otherwise a drifted schema on these is an empty page, not an error. */}
            {[party.error, invites.error, requests.error]
                .filter(Boolean)
                .map((error, i) => (
                    <Muted key={i}>{messageOf(error)}</Muted>
                ))}

            {(invites.data?.length ?? 0) > 0 && (
                <Section title="Party invites">
                    {invites.data?.map((invite) => (
                        <InviteRow
                            key={invite.id}
                            id={invite.id}
                            label={invite.partyName ?? `Party ${invite.partyId}`}
                            message={invite.message}
                        />
                    ))}
                </Section>
            )}

            {incoming.length > 0 && (
                <Section title="Requests">
                    {incoming.map((r) => (
                        <RequestRow key={r.id} request={r} direction="incoming" />
                    ))}
                </Section>
            )}

            <Section title="Friends">
                {friends.isPending && <SkeletonGrid count={3} />}
                {friends.error && <Muted>{messageOf(friends.error)}</Muted>}
                {friends.data && list.length === 0 && (
                    <Muted>
                        No friends yet. Open somebody's profile and press Add friend
                        — or accept a request from a game.
                    </Muted>
                )}
                {list.map((friend) => (
                    <FriendRow key={friend.userId} friend={friend} />
                ))}
            </Section>

            {outgoing.length > 0 && (
                <Section title="Sent">
                    {outgoing.map((r) => (
                        <RequestRow key={r.id} request={r} direction="outgoing" />
                    ))}
                </Section>
            )}
        </div>
    )
}

/** In a game first, then online, away, offline; alphabetical within each. */
function byPresence(a: FriendT, b: FriendT): number {
    const order: PresenceT['status'][] = ['in_game', 'online', 'away', 'offline']
    const rank = order.indexOf(a.presence.status) - order.indexOf(b.presence.status)

    return rank !== 0 ? rank : a.displayName.localeCompare(b.displayName)
}

function Section({
    title,
    children,
}: {
    title: string
    children: React.ReactNode
}) {
    return (
        <section className="flex flex-col gap-1.5">
            <h2 className="text-[11px] font-semibold uppercase tracking-wide text-muted">
                {title}
            </h2>
            <div className="flex flex-col divide-y divide-border rounded-xl border border-border">
                {children}
            </div>
        </section>
    )
}

function Muted({ children }: { children: React.ReactNode }) {
    return <p className="p-3 text-xs text-muted">{children}</p>
}

/** One action button, with its own pending state and error line. */
function useAction<T>(run: () => Promise<T>) {
    const client = useQueryClient()

    return useMutation({
        mutationFn: run,
        onSettled: () => {
            void client.invalidateQueries({ queryKey: ['social'] })
            // Accepting an invite or leaving a party changes the play center's party bar.
            void client.invalidateQueries({ queryKey: ['play'] })
        },
    })
}

function Avatar({ name, url }: { name: string; url: string | null }) {
    // Keyed by URL, so a new URL gets a fresh try.
    const [brokenUrl, setBrokenUrl] = useState<string | null>(null)

    if (url && brokenUrl !== url)
        return (
            <img
                src={url}
                alt=""
                aria-hidden
                onError={() => setBrokenUrl(url)}
                className="size-8 shrink-0 rounded-full object-cover"
            />
        )

    return (
        <span
            aria-hidden
            className="flex size-8 shrink-0 items-center justify-center rounded-full bg-surface-2 text-xs font-semibold"
        >
            {name.slice(0, 1).toUpperCase() || '?'}
        </span>
    )
}

function FriendRow({ friend }: { friend: FriendT }) {
    const [confirm, setConfirm] = useState(false)
    const remove = useAction(() => api.friendRemove(friend.userId))
    const p = friend.presence

    return (
        <div className="flex items-center gap-3 p-2.5">
            <div className="relative">
                <Avatar name={friend.displayName} url={friend.avatarUrl} />
                <span
                    aria-hidden
                    className={`absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full border-2 border-background ${STATUS_TONE[p.status]}`}
                />
            </div>

            <div className="min-w-0 flex-1">
                <Link
                    to={`/view/user/${encodeURIComponent(friend.userId)}`}
                    className="block truncate text-sm font-medium hover:underline"
                >
                    {friend.displayName}
                </Link>
                <p className="truncate text-[11px] text-muted">
                    {presenceLabel(p)}
                    {p.detail ? ` · ${p.detail}` : ''}
                </p>
                {remove.error && (
                    <p className="text-[11px] text-danger">
                        {messageOf(remove.error)}
                    </p>
                )}
            </div>

            {p.serverId != null && (
                <Link
                    to={`/view/server/${p.serverId}`}
                    title={
                        p.joinable
                            ? 'Open the server to join them'
                            : 'Open the server'
                    }
                    className="flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-[11px] hover:border-accent"
                >
                    <FiServer className="size-3" />
                    {p.joinable ? 'Join' : 'Server'}
                </Link>
            )}

            {confirm ? (
                <div className="flex items-center gap-1">
                    <button
                        type="button"
                        disabled={remove.isPending}
                        onClick={() => remove.mutate()}
                        className="rounded-lg bg-danger px-2 py-1 text-[11px] text-white"
                    >
                        Remove
                    </button>
                    <button
                        type="button"
                        onClick={() => setConfirm(false)}
                        className="rounded-lg border border-border px-2 py-1 text-[11px]"
                    >
                        Keep
                    </button>
                </div>
            ) : (
                <button
                    type="button"
                    onClick={() => setConfirm(true)}
                    title="Remove friend"
                    aria-label={`Remove ${friend.displayName}`}
                    className="rounded-lg p-1.5 text-muted hover:text-danger"
                >
                    <FiUserMinus className="size-3.5" />
                </button>
            )}
        </div>
    )
}

function RequestRow({
    request,
    direction,
}: {
    request: FriendRequestT
    direction: 'incoming' | 'outgoing'
}) {
    const accept = useAction(() => api.friendRespond(request.id, true))
    const decline = useAction(() => api.friendRespond(request.id, false))
    const cancel = useAction(() => api.friendCancel(request.id))
    const busy = accept.isPending || decline.isPending || cancel.isPending
    const error = accept.error ?? decline.error ?? cancel.error

    return (
        <div className="flex items-center gap-3 p-2.5">
            <Avatar name={request.displayName} url={null} />
            <div className="min-w-0 flex-1">
                <Link
                    to={`/view/user/${encodeURIComponent(request.userId)}`}
                    className="block truncate text-sm font-medium hover:underline"
                >
                    {request.displayName}
                </Link>
                <p className="text-[11px] text-muted">
                    {direction === 'incoming'
                        ? 'Wants to be friends'
                        : 'Waiting for them'}
                    {' · '}
                    {new Date(request.createdAt).toLocaleDateString()}
                </p>
                {error && (
                    <p className="text-[11px] text-danger">{messageOf(error)}</p>
                )}
            </div>

            {direction === 'incoming' ? (
                <div className="flex items-center gap-1">
                    <button
                        type="button"
                        disabled={busy}
                        onClick={() => accept.mutate()}
                        className="flex items-center gap-1 rounded-lg bg-accent px-2 py-1 text-[11px] text-accent-foreground"
                    >
                        <FiCheck className="size-3" />
                        Accept
                    </button>
                    <button
                        type="button"
                        disabled={busy}
                        onClick={() => decline.mutate()}
                        className="flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-[11px]"
                    >
                        <FiX className="size-3" />
                        Decline
                    </button>
                </div>
            ) : (
                <button
                    type="button"
                    disabled={busy}
                    onClick={() => cancel.mutate()}
                    className="rounded-lg border border-border px-2 py-1 text-[11px]"
                >
                    Withdraw
                </button>
            )}
        </div>
    )
}

export function InviteRow({
    id,
    label,
    message,
}: {
    id: string
    label: string
    message: string | null
}) {
    const accept = useAction(() => api.partyInviteRespond(id, true))
    const decline = useAction(() => api.partyInviteRespond(id, false))
    const busy = accept.isPending || decline.isPending
    const error = accept.error ?? decline.error

    return (
        <div className="flex items-center gap-3 p-2.5">
            <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{label}</p>
                {message && (
                    <p className="truncate text-[11px] text-muted">{message}</p>
                )}
                {error && (
                    <p className="text-[11px] text-danger">{messageOf(error)}</p>
                )}
            </div>
            <button
                type="button"
                disabled={busy}
                onClick={() => accept.mutate()}
                className="rounded-lg bg-accent px-2 py-1 text-[11px] text-accent-foreground"
            >
                Join
            </button>
            <button
                type="button"
                disabled={busy}
                onClick={() => decline.mutate()}
                className="rounded-lg border border-border px-2 py-1 text-[11px]"
            >
                Decline
            </button>
        </div>
    )
}

function PartySection({ party, error }: { party: PartyT | null; error: unknown }) {
    const leave = useAction(() =>
        party ? api.partyLeave(party.id) : Promise.resolve(null)
    )

    if (error) return null

    if (!party)
        return (
            <p className="text-xs text-muted">
                Not in a party. Parties are made in a game or on the website; an
                invite you receive shows up here.
            </p>
        )

    return (
        <Section title="Your party">
            <div className="flex flex-wrap items-center gap-2 p-2.5">
                <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">
                        {party.name ?? `Party ${party.id}`}
                    </p>
                    <p className="text-[11px] text-muted">
                        {party.stage.toLowerCase()} · {party.users}
                        {party.maxUsers ? `/${party.maxUsers}` : ''} players
                        {party.mapName ? ` · ${party.mapName}` : ''}
                    </p>
                    {leave.error && (
                        <p className="text-[11px] text-danger">
                            {messageOf(leave.error)}
                        </p>
                    )}
                </div>
                {party.serverId != null && (
                    <Link
                        to={`/view/server/${party.serverId}`}
                        className="flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-[11px] hover:border-accent"
                    >
                        <FiServer className="size-3" />
                        Server
                    </Link>
                )}
                <button
                    type="button"
                    disabled={leave.isPending}
                    onClick={() => leave.mutate()}
                    className="flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-[11px] hover:border-danger"
                >
                    <FiLogOut className="size-3" />
                    Leave
                </button>
            </div>
            {party.members
                .filter((m) => m.state === 'JOINED')
                .map((m) => (
                    <div
                        key={m.userId}
                        className="flex items-center gap-2 px-2.5 py-1.5"
                    >
                        <Avatar name={m.displayName ?? '?'} url={null} />
                        <span className="min-w-0 flex-1 truncate text-xs">
                            {m.displayName || m.gameName || 'Player'}
                        </span>
                        <span className="text-[10px] uppercase tracking-wide text-muted">
                            {m.role.replace('_', ' ').toLowerCase()}
                            {m.readyAt ? ' · ready' : ''}
                        </span>
                    </div>
                ))}
        </Section>
    )
}
