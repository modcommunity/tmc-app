import { useMemo, type ReactNode } from 'react'
import { Link, useParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import {
    AppAvatarEditor,
    AvatarEditor,
    AvatarStudioHost,
    GroupTabs,
    type StudioHostT,
} from '@modcommunity/shared/avatar'

import { BTN_GHOST, BTN_PRIMARY } from '~/components/play-center'
import { useApiEnv } from '~/lib/api/env'
import { useAuth } from '~/lib/auth/provider'
import { loadGameStudio, loadMemberStudio, writeStudio } from '~/lib/avatar/studio-data'
import { canonicalSlug, isPlatformSlug } from '~/lib/gamespice/games'
import { useGameBySlug } from '~/lib/gamespice/use-game'
import { messageOf } from '~/lib/ipc'
import { ipc } from '~/lib/ipc/commands'

/**
 * The app's side of the shared avatar studio (docs/avatar.md): the global
 * editor at `/avatar` and a game's at `/games/:slug/avatar`. The studio itself
 * (`@modcommunity/shared/avatar`) is the code the website runs; this file
 * gives it what the app has instead of a server: IPC for every read and write,
 * and the model bytes through Rust (`avatar_asset`) so the CSP stays shut.
 */

function Note({ children, back }: { children: ReactNode; back?: string }) {
    return (
        <div className="flex flex-col items-start gap-3 p-4">
            <p className="text-sm text-muted">{children}</p>
            {back && (
                <Link to={back} className={BTN_GHOST}>
                    Back
                </Link>
            )}
        </div>
    )
}

/** The studio's `host`: writes over IPC, models through Rust, nothing else. */
function useHost(appId: number | null): Partial<StudioHostT> & Pick<StudioHostT, 'write'> {
    return useMemo(
        () => ({
            write: writeStudio,
            // The URL is TMC's, unchanged: `loadModel` is what fetches it.
            asset: (url: string) => url,
            loadModel: (url: string) => ipc.avatarAsset(url, appId, true),
            track: () => undefined,
            linkRel: 'noopener noreferrer',
        }),
        [appId]
    )
}

/** Wraps a studio page: the signed-out gate, the site base and the scoped styles. */
function Frame({
    appId,
    back,
    children,
}: {
    appId: number | null
    back: string
    children: (base: string) => ReactNode
}) {
    const { status, signIn } = useAuth()
    const env = useApiEnv()
    const host = useHost(appId)

    if (status === 'loading' || !env) return <Note>Loading…</Note>

    if (status !== 'signedIn') {
        return (
            <div className="flex flex-col items-start gap-3 p-4">
                <p className="text-sm text-muted">Sign in to edit your avatar.</p>
                <button type="button" className={BTN_PRIMARY} onClick={() => void signIn()}>
                    Sign in
                </button>
            </div>
        )
    }

    return (
        <div className="avatar-studio p-3">
            <Link to={back} className="mb-3 block text-xs text-muted hover:text-foreground">
                Back
            </Link>
            <AvatarStudioHost host={host}>{children(new URL(env.base).origin)}</AvatarStudioHost>
        </div>
    )
}

/** `/avatar`: the member's own figure, wearables, emotes and outfits. */
export function MemberAvatarPage() {
    return (
        <Frame appId={null} back="/me">
            {(base) => <MemberBody base={base} />}
        </Frame>
    )
}

function MemberBody({ base }: { base: string }) {
    const q = useQuery({
        queryKey: ['avatar', 'member', base],
        queryFn: () => loadMemberStudio(base),
        staleTime: 0,
    })

    if (q.isPending) return <Note>Loading…</Note>
    if (q.error) return <Note>{messageOf(q.error)}</Note>

    return <AvatarEditor {...q.data} />
}

/** `/games/:slug/avatar`: the avatar, wearables and emotes in one game. */
export function GameAvatarPage() {
    const { slug: raw } = useParams()
    const slug = raw ? canonicalSlug(raw) : undefined
    const game = useGameBySlug(slug)

    if (!slug || !isPlatformSlug(slug)) return <Note back="/games">That is not a GameSpice game.</Note>
    if (game.isPending) return <Note>Loading…</Note>
    if (!game.data) return <Note back="/games">No such game.</Note>

    const appId = game.data.id

    return (
        <Frame appId={appId} back={`/games/${slug}`}>
            {(base) => <GameBody appId={appId} base={base} />}
        </Frame>
    )
}

function GameBody({ appId, base }: { appId: number; base: string }) {
    const q = useQuery({
        queryKey: ['avatar', 'game', appId],
        queryFn: () => loadGameStudio(appId),
        staleTime: 0,
    })

    if (q.isPending) return <Note>Loading…</Note>
    if (q.error) return <Note>{messageOf(q.error)}</Note>
    if (!q.data) return <Note>This game has avatars switched off.</Note>

    const { library, me, actions, points, manifest } = q.data

    return (
        <AppAvatarEditor
            appId={appId}
            library={library}
            tabs={GroupTabs(library, 'en')}
            initial={me}
            actions={actions}
            points={points}
            tmcAvatars={`${base}/avatars`}
            manifest={manifest}
        />
    )
}
