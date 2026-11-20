import { z } from 'zod'

import { call } from '~/lib/ipc'
import {
    AppListQuerySchema,
    AvatarResponseSchema,
    AvatarActionsResponseSchema,
    AvatarWearablesResponseSchema,
    AvatarSchemaResponseSchema,
    type AvatarDocT,
    AppListResponseSchema,
    BrowseQuerySchema,
    BrowseResponseSchema,
    ContentDetailSchema,
    FacetsResponseSchema,
    FriendListResponse,
    FriendRequestSentResponse,
    FriendRequestsResponse,
    InstallListResponse,
    InstallSchema,
    MeResponseSchema,
    PartyInviteRespondResponse,
    PartyInvitesResponse,
    PartyLeaveResponse,
    PartyMineResponse,
    PlayLaunchResponseSchema,
    PlayPageEnvelopeSchema,
    PlayPageQuerySchema,
    PlayPlayableResponseSchema,
    PlayQuickMatchResponseSchema,
    ReportCreateResponse,
    ReviewListResponse,
    ReviewVoteResponse,
    ReviewWriteResponse,
    ServerLookupResponseSchema,
    StatDefsResponse,
    StatsMeResponse,
    StatsTopResponse,
    SubscriptionSyncResponse,
    UserSettingsSchema,
    type AppListQueryT,
    type BrowseQueryT,
    type ContentKindT,
    type InstallOptionsT,
    type PlayModeT,
    type PlayOptionValuesT,
    type PlayPageQueryT,
    type SubKindT,
    type UserSettingsT,
    WorkshopCatalogResponse,
    WorkshopItemWriteResponse,
    WorkshopMineResponse,
    WorkshopOwnedResponse,
    type WorkshopCatalogQueryT,
    WorkshopScopeResponse,
    PointsSummaryResponse,
    AchievementsResponse,
    MeStatsResponse,
    StoreDirectoryResponse,
    StoreFrontResponse,
    StoreLoadoutResponse,
    StorePurchaseResponse,
    StorePurchasesResponse,
    type StoreCurrencyT,
    AvatarOutfitsResponse,
} from './contract'

/**
 * The website API, as the app sees it.
 *
 * There is no `fetch` anywhere in the frontend. Requests go out through Rust,
 * which owns the bearer token and the base URL — so the webview cannot leak a
 * credential it never has, and cannot be pointed at a different host by
 * anything that manages to run inside it.
 *
 * Every response is parsed. `BrowseResponseSchema.parse` is what makes the rest
 * of the app's types real rather than an assertion.
 */

const RawJson = z.unknown()

/** A document whose shape the avatar studio checks itself. */
const StudioDocSchema = z.record(z.string(), z.unknown())

async function get<S extends z.ZodType>(
    path: string,
    schema: S,
    query?: Record<string, QueryValue>,
    auth = false
): Promise<z.infer<S>> {
    const raw = await call('api_get', RawJson, {
        path,
        query: query ? toPairs(query) : undefined,
        auth,
    })

    return parseOrThrow(path, schema, raw)
}

async function send<S extends z.ZodType>(
    method: 'POST' | 'PATCH' | 'PUT' | 'DELETE',
    path: string,
    schema: S,
    body?: unknown,
    auth = true
): Promise<z.infer<S>> {
    const raw = await call('api_send', RawJson, { method, path, body, auth })

    return parseOrThrow(path, schema, raw)
}

function parseOrThrow<S extends z.ZodType>(
    path: string,
    schema: S,
    raw: unknown
): z.infer<S> {
    const parsed = schema.safeParse(raw)

    if (!parsed.success) {
        /*
         * A shape mismatch is a CONTRACT drift — this file and
         * website-city's `src/types/app-api/contract.ts` have diverged, or the
         * server is older than the app. Both are bugs worth failing loudly for
         * rather than papering over with optional chaining at every call site.
         */
        console.error(`[api] ${path} did not match the contract`, parsed.error, raw)

        throw new Error(
            'The server sent something this version of the app does not understand. Try updating.'
        )
    }

    return parsed.data
}

/** Values a query string can carry. Anything else is a programming error. */
type QueryValue = string | number | boolean | null | undefined | (string | number)[]

/**
 * Flatten to the `[key, value][]` the Rust side encodes.
 *
 * Arrays repeat their key, matching the server's `parseQuery`. `undefined` and
 * `false` are dropped rather than sent: the API's booleans are all opt-IN
 * (`nsfw`, `onlineOnly`), so an explicit `false` and an absent key mean the
 * same thing and omitting keeps the URL short.
 */
function toPairs(query: Record<string, QueryValue>): [string, string][] {
    const out: [string, string][] = []

    for (const [key, value] of Object.entries(query)) {
        if (value == null || value === false || value === '') continue

        if (Array.isArray(value)) {
            for (const item of value) out.push([key, String(item)])
            continue
        }

        out.push([key, String(value)])
    }

    return out
}

export const api = {
    me: () => get('/me', MeResponseSchema, undefined, true),

    updateSettings: (patch: Partial<UserSettingsT>) =>
        send('PATCH', '/me', UserSettingsSchema, patch),

    browse: (query: BrowseQueryT) =>
        get('/browse', BrowseResponseSchema, BrowseQuerySchema.parse(query)),

    content: (kind: ContentKindT, id: string) =>
        get(
            `/content/${encodeURIComponent(kind)}/${encodeURIComponent(id)}`,
            ContentDetailSchema
        ),

    avatar: () => get('/avatar', AvatarResponseSchema, undefined, true),

    avatarSchema: () => get('/avatar/schema', AvatarSchemaResponseSchema),

    avatarSave: (doc: AvatarDocT) => send('PUT', '/avatar', z.unknown(), doc),

    avatarActions: () =>
        get('/avatar/actions', AvatarActionsResponseSchema, undefined, true),

    avatarActionsSave: (kind: 'EMOTE' | 'POSE' | 'GESTURE', actionIds: number[]) =>
        send('PUT', '/avatar/actions', z.unknown(), { kind, actionIds }),

    /** `appId` narrows to what that game renders and adds the game's own wearables. */
    avatarWearables: (appId?: number) =>
        get(
            '/avatar/wearables',
            AvatarWearablesResponseSchema,
            appId === undefined ? undefined : { appId: String(appId) },
            true
        ),

    avatarWearableEquip: (wearableId: number, equipped: boolean, appId?: number) =>
        send('PUT', '/avatar/wearables', z.unknown(), { wearableId, equipped, appId }),

    // ---- Avatar studio (`@modcommunity/shared/avatar`): the documents are handed
    // on as the server sent them; the studio's own guards (ParseManifest and
    // friends) read their shape, so this layer only insists on "an object".

    /** One of the member's avatar documents (`avatar`, `avatar/schema`, `avatar/wearables`, `avatar/actions`, `me/points`). */
    studioGet: (path: string, query?: Record<string, QueryValue>) =>
        get(path, StudioDocSchema, query, true),

    /** One studio write to `/api/app/v1` (`avatar`, `avatar/wearables`, `avatar/actions`, `avatar/outfits`). */
    studioPut: (path: string, body: unknown) => send('PUT', path, z.unknown(), body),

    // ---- GameSpice hub reads (shapes in the contract, section GameSpice hub)

    /** Every Workshop subscription of the member, across games. */
    workshopMine: () => get('/workshop/mine', WorkshopMineResponse, undefined, true),

    /** Which games a subscription loads into: every game, or one game on or off. */
    workshopScope: (
        subscriptionId: string,
        change: { all: boolean } | { game: string; on: boolean }
    ) => send('POST', '/workshop/scope', WorkshopScopeResponse, { subscriptionId, ...change }),

    /** The whole Workshop: search, kind and game filters, sort, page; plus what is subscribed. */
    workshopCatalog: (q: Partial<WorkshopCatalogQueryT> = {}) =>
        get('/workshop/catalog', WorkshopCatalogResponse, q, true),

    /** The member's own packs that are, or can be, offered in the Workshop. */
    workshopOwned: () => get('/workshop/owned', WorkshopOwnedResponse, undefined, true),

    /** Owner edit: name, summary, and where it is offered (`none`/`any`/`listed` + game ids). */
    workshopItemPatch: (patch: {
        id: number
        name?: string
        summary?: string
        compat?: 'none' | 'any' | 'listed'
        games?: number[]
    }) => send('PATCH', '/workshop/item', WorkshopItemWriteResponse, patch),

    /** Owner delete. */
    workshopItemDelete: (id: number) =>
        send('DELETE', '/workshop/item', WorkshopItemWriteResponse, { id }),

    /** Points in every pool. */
    myPoints: () => get('/me/points', PointsSummaryResponse, undefined, true),

    /** The member's saved outfits; 404 (avatars off) surfaces as an error. */
    avatarOutfits: () => get('/avatar/outfits', AvatarOutfitsResponse, undefined, true),

    facets: (kind: ContentKindT, online = true) =>
        get('/facets', FacetsResponseSchema, {
            kind,
            ...(kind === 'server' ? { online } : {}),
        }),

    /**
     * Which listed server is at an address.
     *
     * What a `gs://play/<host>:<port>` link resolves through. The caller
     * already holds the address — that is what a link IS — so this discloses
     * nothing the browser does not; what it adds is which game is running
     * there, which is the fact a join screen needs before it can draw a button.
     *
     * A host running two servers answers with both, newest-and-busiest first,
     * and the screen draws a choice rather than guessing between them.
     */
    serverLookup: (host: string, port?: number) =>
        get('/servers/lookup', ServerLookupResponseSchema, { host, port }),

    // ------------------------------------------------------------------ Apps
    /**
     * The whole app catalogue — the Apps tab's only source.
     *
     * Public, like `/browse`: signing in changes nothing about which games
     * exist. It is a separate endpoint from `/facets` because facets answer
     * "which games have assets", scoped to one kind and counted per kind, while
     * this answers "which games are there" and carries the play vocabulary
     * `/facets` has no reason to know about.
     */
    apps: (query: AppListQueryT = {}) =>
        get('/apps', AppListResponseSchema, AppListQuerySchema.parse(query)),

    /**
     * Resolve a launch, server-side.
     *
     * Answers `null` for every refusal, which is the whole point of asking: the
     * app never decides whether a game can be started by reading columns off a
     * catalogue row, so the button that is drawn and the launch that happens
     * agree by construction.
     *
     * This covers only the modes the SERVER owns. Starting a copy of the game
     * installed on this machine is the device's own business and never comes
     * through here — see `ipc.playLaunchNative`.
     */
    playLaunch: (input: {
        appId: number
        mode: PlayModeT
        serverId?: number
        locale?: string
        options?: PlayOptionValuesT
    }) =>
        send('POST', '/play/launch', PlayLaunchResponseSchema, {
            locale: 'en',
            ...input,
        }),

    // ------------------------------------------------------------ Play center
    /**
     * One page of one tab of the play center. Null when the feature is off.
     *
     * Public with a bearer attached when there is one: the party and the
     * invites tab are the viewer's own, and a signed-out caller simply gets
     * `party: null`, so this never refuses on a missing sign-in.
     */
    playPage: (query: PlayPageQueryT) =>
        get('/play/page', PlayPageEnvelopeSchema, PlayPageQuerySchema.parse(query)),

    /** Which games have a Play button. `enabled: false` when the feature is off. */
    playPlayable: () => get('/play/playable', PlayPlayableResponseSchema),

    /**
     * The best server for one game, right now. Null when the feature is off or
     * the game is not listed. A POST because every press is a fresh search;
     * `auth` is optional on the site, so a signed-out app can ask too.
     */
    playQuickMatch: (appId: number, rotation = false) =>
        send(
            'POST',
            '/play/quick-match',
            PlayQuickMatchResponseSchema,
            { appId, rotation },
            false
        ),

    /**
     * What people said about one item.
     *
     * Public, so a signed-out app shows the same reviews a browser does. Read
     * only: writing one needs the website's own rate limiting, edit window and
     * moderation hooks, and half of those living in a second place is how they
     * drift.
     */
    reviews: (
        kind: ContentKindT,
        id: number,
        options?: {
            sort?: 'recent' | 'helpful' | 'rating'
            cursor?: string | null
        }
    ) =>
        get('/reviews', ReviewListResponse, {
            kind,
            id,
            sort: options?.sort ?? 'recent',
            cursor: options?.cursor ?? undefined,
        }),

    /**
     * Leave or edit a review.
     *
     * One per person per item: a repeat call updates, because the model's
     * unique constraint makes a second insert an error rather than a second
     * review.
     */
    writeReview: (input: {
        kind: ContentKindT
        id: number
        rating?: number | null
        content?: string | null
    }) => send('POST', '/reviews/write', ReviewWriteResponse, input),

    deleteReview: (id: number) =>
        send('DELETE', '/reviews/write', z.object({ deleted: z.literal(true) }), {
            id,
        }),

    /** Mark a review helpful, or `null` to take the vote back. */
    voteReview: (id: number, positive: boolean | null) =>
        send('PUT', '/reviews/write', ReviewVoteResponse, { id, positive }),

    /** Report an item. Lands in the same moderation queue as the website's form. */
    report: (input: {
        kind: ContentKindT
        id: number
        type?: 'SPAM' | 'OTHER'
        title: string
        content: string
    }) => send('POST', '/report', ReportCreateResponse, input),

    // -------------------------------------------------------- Subscriptions
    /**
     * The sync endpoint, called DIRECTLY only by screens that want a fresh
     * answer without side effects.
     *
     * The device's own sync loop does not come through here — it runs in Rust
     * (`library_sync`), because the pass that follows it writes to the user's
     * game folders and the webview must not be the thing that decides to.
     */
    subscriptions: (since?: string) =>
        get(
            '/subscriptions',
            SubscriptionSyncResponse,
            { since, limit: 200 },
            true
        ),

    /** Subscribe or unsubscribe from the app. Omit `subscribed` to toggle. */
    subscribe: (kind: SubKindT, itemId: number, subscribed?: boolean) =>
        send(
            'POST',
            '/subscriptions',
            z.object({
                subscribed: z.boolean(),
                propagated: z.number(),
                skipped: z.number(),
            }),
            { kind, itemId, subscribed }
        ),

    /** Auto-update / notify / pause, as a partial. */
    subscriptionPrefs: (
        kind: SubKindT,
        itemId: number,
        prefs: {
            autoUpdate?: boolean
            notifyUpdates?: boolean
            paused?: boolean
        }
    ) =>
        send('POST', '/subscriptions/prefs', z.object({ ok: z.boolean() }), {
            kind,
            itemId,
            ...prefs,
        }),

    // --------------------------------------------------------------- Installs
    installs: (appId?: number) =>
        get('/installs', InstallListResponse, { appId }, true),

    installCreate: (input: {
        appId: number
        name: string
        description?: string
        gameVersion?: string
        loader?: string
        isDefault?: boolean
        launch?: {
            args?: string[]
            env?: Record<string, string>
            options?: InstallOptionsT
        }
    }) => send('POST', '/installs', InstallSchema, input),

    installUpdate: (input: {
        id: number
        name?: string
        description?: string | null
        gameVersion?: string | null
        loader?: string | null
        isDefault?: boolean
        launch?: {
            args?: string[]
            env?: Record<string, string>
            options?: InstallOptionsT
        }
        playedSeconds?: number
    }) => send('PATCH', '/installs', InstallSchema, input),

    installDelete: (id: number) =>
        send('DELETE', '/installs', z.object({ deleted: z.boolean() }), { id }),

    installAddItem: (installId: number, kind: SubKindT, itemId: number) =>
        send('POST', '/installs/items', InstallSchema, {
            installId,
            kind,
            itemId,
        }),

    installRemoveItem: (installId: number, kind: SubKindT, itemId: number) =>
        send('DELETE', '/installs/items', InstallSchema, {
            installId,
            kind,
            itemId,
        }),

    installPatchItem: (
        installId: number,
        kind: SubKindT,
        itemId: number,
        patch: { enabled?: boolean; order?: number }
    ) =>
        send('PATCH', '/installs/items', InstallSchema, {
            installId,
            kind,
            itemId,
            ...patch,
        }),

    // ------------------------------------------------ Friends, parties, stats
    /*
     * The game backbone's player routes (website-city `docs/api/app-social.md`).
     * Every one is signed-in only, and every one accepts the device token Rust
     * holds — so the webview still never sees a credential, and a game running
     * in the web player and this app share the site's rate-limit buckets rather
     * than each getting their own.
     *
     * Schemas are the contract's "Social" section; `./social` keeps the UI's
     * helpers.
     */

    friends: () => get('/friends', FriendListResponse, undefined, true),

    friendRequests: () =>
        get('/friends/requests', FriendRequestsResponse, undefined, true),

    /** Asking somebody who already asked you accepts them — the site's rule. */
    friendRequest: (userId: string) =>
        send('POST', '/friends/request', FriendRequestSentResponse, { userId }),

    friendRespond: (requestId: number, accept: boolean) =>
        send('POST', '/friends/respond', z.null(), { requestId, accept }),

    friendCancel: (requestId: number) =>
        send('POST', '/friends/cancel', z.null(), { requestId }),

    friendRemove: (userId: string) =>
        send('POST', '/friends/remove', z.null(), { userId }),

    /** The caller's party, or null. */
    partyMine: () => get('/party/mine', PartyMineResponse, undefined, true),

    partyInvites: () =>
        get('/party/invites', PartyInvitesResponse, undefined, true),

    /** Accepting also JOINS, so capacity and bans still decide. */
    partyInviteRespond: (inviteId: string, accept: boolean) =>
        send('POST', '/party/invite/respond', PartyInviteRespondResponse, {
            inviteId,
            accept,
        }),

    partyLeave: (id: string) =>
        send('POST', '/party/leave', PartyLeaveResponse, { id }),

    /**
     * One stat of one game as a ranking, with the signed-in member's own row.
     * Signed-in only: `self` is derived from the member's app-scoped key.
     */
    statsTop: (app: number, stat: string, limit = 25, offset = 0) =>
        get('/stats/top', StatsTopResponse, { app, stat, limit, offset }, true),

    /** Every value the signed-in member holds in one game. */
    statsMe: (app: number) => get('/stats/me', StatsMeResponse, { app }, true),

    /** Every visible stat one game declared — what the leaderboard picker lists. */
    statsDefs: (app: number) => get('/stats/defs', StatDefsResponse, { app }, true),

    /** Every game whose own store is open, most items first. Signed-out works. */
    storeDirectory: () => get('/store/directory', StoreDirectoryResponse, undefined, false),

    /** One game's store: items, the member's wallet, what they own. */
    storeFront: (appId: number) => get('/store/front', StoreFrontResponse, { appId }, false),

    /**
     * Buy one item with points (or the game's own points). `idemKey` is made
     * once per confirmed buy so a retried request cannot charge twice;
     * `price`/`currency` are what the member saw, and the server answers 409
     * `changed` if that is no longer true.
     */
    storePurchase: (req: { itemId: number; idemKey: string; price: number; currency: StoreCurrencyT }) =>
        send('POST', '/store/purchase', StorePurchaseResponse, req),

    /** What the member has equipped in one game. */
    storeLoadout: (appId: number) => get('/store/loadout', StoreLoadoutResponse, { appId }, true),

    /** Equip `key` in `slot` (or clear it with `null`). */
    storeLoadoutPut: (req: { appId: number; slot: string; key: string | null; store?: string }) =>
        send('PUT', '/store/loadout', StoreLoadoutResponse, req),

    /** The member's purchases across every store, newest first. */
    storePurchases: (cursor?: number, limit = 50) =>
        get('/store/purchases', StorePurchasesResponse, { limit, cursor }, true),

    /** Days played per game, the 30-day series and achievement totals. */
    meStats: () => get('/me/stats', MeStatsResponse, undefined, true),

    /** One game's achievements: unlocked (with time) and locked. */
    statsAchievements: (app: number) =>
        get('/stats/achievements', AchievementsResponse, { app }, true),
}
