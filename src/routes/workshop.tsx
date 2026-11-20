import { Link } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Button } from '@modcommunity/shared'

import { api } from '~/lib/api/client'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'

/**
 * **Workshop** — the packs a member has subscribed to, across every GameSpice
 * game, and whether each applies to all games or only the listed ones.
 * Finding new packs is the ordinary Assets browse (subscribe from there); this
 * screen is where they are managed. Signed in only: the list is the member's.
 */
export default function WorkshopRoute() {
    const { status, signIn } = useAuth()
    const qc = useQueryClient()
    const mine = useQuery({
        queryKey: ['gamespice', 'workshop'],
        queryFn: () => api.workshopMine(),
        enabled: status === 'signedIn',
    })
    const scope = useMutation({
        mutationFn: (v: { id: string; all: boolean }) => api.workshopScope(v.id, { all: v.all }),
        onSuccess: () => qc.invalidateQueries({ queryKey: ['gamespice', 'workshop'] }),
    })

    if (status !== 'signedIn')
        return (
            <div className="flex flex-col items-start gap-3 p-4">
                <h1 className="text-lg font-bold">Workshop</h1>
                <p className="text-sm text-muted">Sign in to see the packs you have subscribed to.</p>
                <Button btnType="primary" onClick={() => void signIn()}>
                    Sign in
                </Button>
            </div>
        )

    return (
        <div className="flex flex-col gap-4 p-4">
            <header>
                <h1 className="text-lg font-bold">Workshop</h1>
                <p className="text-xs text-muted">
                    Packs you subscribed to. Mods and sandboxes live in your{' '}
                    <Link to="/library" className="underline">Library</Link>; find more in{' '}
                    <Link to="/browse/asset" className="underline">Assets</Link>.
                </p>
            </header>

            {mine.isPending && <p className="text-sm text-muted">Loading…</p>}
            {mine.isError && (
                <p role="alert" className="text-sm text-danger">{messageOf(mine.error)}</p>
            )}
            {scope.isError && (
                <p role="alert" className="text-sm text-danger">{messageOf(scope.error)}</p>
            )}
            {mine.data?.items.length === 0 && (
                <p className="text-sm text-muted">You have no Workshop subscriptions yet.</p>
            )}

            <ul className="flex flex-col gap-2">
                {mine.data?.items.map((it) => (
                    <li
                        key={it.subscriptionId}
                        className="flex items-center gap-3 rounded-xl border border-border bg-surface p-3"
                    >
                        {it.image ? (
                            <img src={it.image} alt="" className="size-12 rounded-lg object-cover" />
                        ) : (
                            <div className="size-12 rounded-lg bg-surface-2" />
                        )}
                        <div className="min-w-0 flex-1">
                            <Link to={`/view/asset/${it.assetId}`} className="block truncate text-sm font-semibold">
                                {it.name}
                            </Link>
                            <p className="text-[11px] text-muted">
                                {it.scope === 'all' ? 'Applies to every game' : 'Only the games you picked'}
                                {it.via ? ` · via ${it.via.name}` : ''}
                                {it.available === false ? ' · unavailable' : ''}
                            </p>
                        </div>
                        <Button
                            btnType="secondary"
                            disabled={scope.isPending}
                            onClick={() => scope.mutate({ id: it.subscriptionId, all: it.scope !== 'all' })}
                        >
                            {it.scope === 'all' ? 'Limit to listed' : 'Use in all games'}
                        </Button>
                    </li>
                ))}
            </ul>
        </div>
    )
}
