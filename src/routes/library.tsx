import { Link, useSearchParams } from 'react-router-dom'
import { FiDownload, FiLayers, FiPackage } from 'react-icons/fi'

import { useAuth } from '~/lib/auth/provider'
import { useLibrary } from '~/lib/library/provider'
import LibraryGames from '~/components/library-games'
import LibraryContent from '~/components/library-content'

/**
 * **Library** — this device's half of the app.
 *
 * Two views, because the screen was answering the wrong question. It used to be
 * a list of subscriptions, which tells somebody what they subscribed to; the
 * question people open a mod manager for is *what have I got installed, and how
 * do I play it*. That is now the view it opens on, and the subscription list is
 * the second tab rather than the whole screen.
 *
 * The split is not cosmetic. The two lists come from different places and mean
 * different things — Games is assembled from this machine's sandboxes and
 * configured folders, Content is the account's subscriptions mirrored down —
 * and one list holding both would have to explain, per row, which kind of thing
 * it was.
 */

type View = 'games' | 'content'

export default function LibraryRoute() {
    const { status } = useAuth()
    const { rows } = useLibrary()
    const [params, setParams] = useSearchParams()

    /*
     * In the URL, like every other durable view choice in this app: a link to
     * the content view has to survive a reload and the back button, and it is
     * what lets another screen point at one half of this one.
     */
    const view: View = params.get('view') === 'content' ? 'content' : 'games'

    const setView = (next: View) => {
        const updated = new URLSearchParams(params)

        if (next === 'games') updated.delete('view')
        else updated.set('view', next)

        setParams(updated, { replace: true })
    }

    /*
     * The Games view works signed out — sandboxes, game folders and play time
     * are all facts about this machine, and a user who has not signed in can
     * still mod a game they own. Only the subscription list needs an account,
     * so only that half asks for one.
     */
    if (status !== 'signedIn' && view === 'content')
        return (
            <div className="flex flex-col gap-3 p-3">
                <Header view={view} onChange={setView} count={0} />

                <div className="rounded-xl border border-border p-6 text-center">
                    <p className="text-sm text-muted">
                        Sign in to see the mods and assets you subscribed to.
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

    return (
        <div className="flex flex-col gap-3 p-3">
            <Header view={view} onChange={setView} count={rows.length} />

            {view === 'games' ? <LibraryGames /> : <LibraryContent />}
        </div>
    )
}

function Header({
    view,
    onChange,
    count,
}: {
    view: View
    onChange: (next: View) => void
    count: number
}) {
    return (
        <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-1 rounded-lg border border-border p-0.5">
                <Tab
                    active={view === 'games'}
                    onClick={() => onChange('games')}
                    icon={FiLayers}
                    label="Games"
                />
                <Tab
                    active={view === 'content'}
                    onClick={() => onChange('content')}
                    icon={FiPackage}
                    label="Subscribed"
                    badge={count || undefined}
                />
            </div>

            <Link
                to="/downloads"
                className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-xs hover:border-accent"
            >
                <FiDownload className="size-3" />
                Downloads
            </Link>
        </div>
    )
}

function Tab({
    active,
    onClick,
    icon: Icon,
    label,
    badge,
}: {
    active: boolean
    onClick: () => void
    icon: typeof FiLayers
    label: string
    badge?: number
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            aria-pressed={active}
            className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs transition ${
                active ? 'bg-accent text-accent-foreground' : 'text-muted'
            }`}
        >
            <Icon className="size-3.5" />
            {label}
            {badge !== undefined && <span className="opacity-70">{badge}</span>}
        </button>
    )
}
