import {
    AbsoluteUrl,
    AppPoints,
    ParseManifest,
    ParseOutfits,
    type ActionsT,
    type AppAvatarT,
    type AvatarLibraryT,
    type AvatarManifestT,
    type AvatarSchemaT,
    type MemberAvatarT,
    type MemberPointsT,
    type OutfitT,
    type PointsPoolT,
    type StudioWriteT,
    type WearablesT,
    type WriteResultT,
} from '@modcommunity/shared/avatar'

import { api } from '~/lib/api/client'
import { ipc } from '~/lib/ipc/commands'
import { isIpcError, messageOf } from '~/lib/ipc'

/**
 * The app's half of the avatar studio (docs/avatar.md): what the studio reads
 * and writes, over IPC. Nothing here names a host or holds a credential; the
 * studio's own guards read the shape of each document.
 */

/** Everything the global editor edits. */
export type MemberStudioT = {
    avatar: MemberAvatarT
    schema: AvatarSchemaT
    wearables: WearablesT
    actions: ActionsT
    /** Null when TMC could not list outfits (an older TMC). */
    outfits: OutfitT[] | null
    /** Null shows the 2D render instead. */
    manifest: AvatarManifestT | null
}

/** Everything the per-game editor edits. */
export type GameStudioT = {
    library: AvatarLibraryT
    me: AppAvatarT
    actions: ActionsT | null
    points: PointsPoolT | null
    manifest: AvatarManifestT | null
}

/** A document read, cast to what the studio types it as (the server owns the shape). */
function as<T>(doc: unknown): T {
    return doc as T
}

async function manifest(): Promise<AvatarManifestT | null> {
    try {
        return ParseManifest(await ipc.avatarManifest())
    } catch {
        // Only the 3D stage needs it; without it the editor still edits.
        return null
    }
}

/** `/avatar` and friends: the member's figure, with TMC's relative file paths made absolute. */
export async function loadMemberStudio(base: string): Promise<MemberStudioT> {
    const avatar = as<MemberAvatarT>(await api.studioGet('/avatar'))
    const [schema, wearables, actions, outfits, man] = await Promise.all([
        api.studioGet('/avatar/schema'),
        api.studioGet('/avatar/wearables'),
        api.studioGet('/avatar/actions'),
        api
            .studioGet('/avatar/outfits')
            .then((r) => ParseOutfits(JSON.stringify(r.outfits)))
            .catch(() => null),
        manifest(),
    ])
    const url = avatar.twoD ? AbsoluteUrl(avatar.twoD.url, base) : null
    const w = as<WearablesT>(wearables)

    return {
        avatar: { ...avatar, twoD: avatar.twoD && url ? { ...avatar.twoD, url } : null },
        schema: as<AvatarSchemaT>(schema),
        wearables: {
            ...w,
            wearables: w.wearables.map((x) => ({
                ...x,
                url: x.url ? AbsoluteUrl(x.url, base) : null,
            })),
        },
        actions: as<ActionsT>(actions),
        outfits,
        manifest: man,
    }
}

/** One game: null when its avatars are off. */
export async function loadGameStudio(appId: number): Promise<GameStudioT | null> {
    const library = await ipc.avatarLibrary(appId)

    if (!library) return null

    const me = as<AppAvatarT>(await ipc.avatarMe(appId))
    const [actions, points, man] = await Promise.all([
        api.studioGet('/avatar/actions', { appId }).then(as<ActionsT>).catch(() => null),
        api.studioGet('/me/points').then((p) => AppPoints(as<MemberPointsT>(p), appId)).catch(() => null),
        manifest(),
    ])

    return { library: as<AvatarLibraryT>(library), me, actions, points, manifest: man }
}

/** Where each studio write goes: one `/api/app/v1` path, or the game's own avatar route. */
const WRITES: Record<Exclude<StudioWriteT, 'app-avatar'>, string> = {
    avatar: '/avatar',
    wearables: '/avatar/wearables',
    actions: '/avatar/actions',
    outfits: '/avatar/outfits',
}

/** The studio's status for a failure: 401 signed out, 0 no network, anything else an error. */
export function statusOf(err: unknown): number {
    if (!isIpcError(err)) return 0

    switch (err.code) {
        case 'not_authenticated':
        case 'auth_rejected':
            return 401
        case 'network':
            return 0
        default:
            return 400
    }
}

export async function writeStudio(what: StudioWriteT, body: unknown): Promise<WriteResultT<unknown>> {
    try {
        const data =
            what === 'app-avatar'
                ? await ipc.avatarMePut(body)
                : await api.studioPut(WRITES[what], body)

        return { ok: true, data }
    } catch (err) {
        return { ok: false, status: statusOf(err), message: messageOf(err) }
    }
}
