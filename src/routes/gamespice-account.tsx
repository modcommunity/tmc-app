import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { Button } from '@modcommunity/shared'

import { api } from '~/lib/api/client'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'
import { BTN_GHOST } from '~/components/play-center'

/**
 * **My GameSpice** — points and saved outfits. Both are the member's own, so
 * nothing is requested, or drawn, while signed out.
 */
export default function GameSpiceAccountRoute() {
    const { status, user, signIn } = useAuth()
    const on = status === 'signedIn'
    const points = useQuery({ queryKey: ['gamespice', 'points'], queryFn: () => api.myPoints(), enabled: on })
    const appIds = (points.data?.apps ?? []).map((a) => a.appId)
    const names = useQuery({
        queryKey: ['gamespice', 'app-names', appIds.join(',')],
        queryFn: () => api.apps({ ids: appIds }),
        enabled: appIds.length > 0,
        staleTime: 300_000,
    })
    const nameOf = (id: number) => names.data?.apps.find((a) => a.id === id)?.name ?? `App ${id}`
    const outfits = useQuery({ queryKey: ['gamespice', 'outfits'], queryFn: () => api.avatarOutfits(), enabled: on })

    if (!on)
        return (
            <div className="flex flex-col items-start gap-3 p-4">
                <h1 className="text-lg font-bold">My GameSpice</h1>
                <p className="text-sm text-muted">Sign in to see your points and outfits.</p>
                <Button btnType="primary" onClick={() => void signIn()}>
                    Sign in
                </Button>
            </div>
        )

    return (
        <div className="flex flex-col gap-6 p-4">
            <header>
                <h1 className="text-lg font-bold">{user?.name ?? user?.username ?? 'My GameSpice'}</h1>
                <p className="text-xs text-muted">
                    <Link to="/settings/account" className="underline">Account settings</Link>
                </p>
            </header>

            <section className="flex flex-col gap-2">
                <h2 className="text-sm font-semibold">Points</h2>
                {points.isPending && <p className="text-sm text-muted">Loading…</p>}
                {points.isError && <p role="alert" className="text-sm text-danger">{messageOf(points.error)}</p>}
                {points.data && (
                    <>
                        <p className="text-2xl font-extrabold">{points.data.total}</p>
                        <p className="text-xs text-muted">
                            Site: {points.data.site.available} available ({points.data.site.earned} earned,{' '}
                            {points.data.site.spent} spent)
                        </p>
                        <ul className="flex flex-col gap-1 text-xs">
                            {points.data.apps.map((a) => (
                                <li key={a.appId} className="flex justify-between rounded-lg border border-border px-3 py-1.5">
                                    <span>{nameOf(a.appId)}{a.trusted ? '' : ' (unverified)'}</span>
                                    <span>{a.available} available</span>
                                </li>
                            ))}
                        </ul>
                    </>
                )}
            </section>

            <section className="flex flex-col gap-2">
                <div className="flex items-center justify-between">
                    <h2 className="text-sm font-semibold">Outfits</h2>
                    <Link to="/avatar" className={BTN_GHOST}>Avatar</Link>
                </div>
                {outfits.isPending && <p className="text-sm text-muted">Loading…</p>}
                {outfits.isError && <p role="alert" className="text-sm text-danger">{messageOf(outfits.error)}</p>}
                {outfits.data?.outfits.length === 0 && <p className="text-sm text-muted">No saved outfits yet.</p>}
                <ul className="flex flex-col gap-1 text-sm">
                    {outfits.data?.outfits.map((o) => (
                        <li key={o.id} className="rounded-lg border border-border px-3 py-1.5">
                            {o.name}
                            {o.savedAt ? <span className="ml-2 text-[11px] text-muted">{o.savedAt}</span> : null}
                        </li>
                    ))}
                </ul>
            </section>
        </div>
    )
}
