import { z } from 'zod'

import { call } from '~/lib/ipc'
import {
    AppListQuerySchema,
    AppListResponseSchema,
    BrowseQuerySchema,
    BrowseResponseSchema,
    ContentDetailSchema,
    FacetsResponseSchema,
    InstallListResponse,
    InstallSchema,
    MeResponseSchema,
    PlayLaunchResponseSchema,
    ReportCreateResponse,
    ReviewListResponse,
    ReviewVoteResponse,
    ReviewWriteResponse,
    SubscriptionSyncResponse,
    UserSettingsSchema,
    type AppListQueryT,
    type BrowseQueryT,
    type ContentKindT,
    type InstallOptionsT,
    type PlayModeT,
    type PlayOptionValuesT,
    type SubKindT,
    type UserSettingsT,
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

async function get<S extends z.ZodTypeAny>(
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

async function send<S extends z.ZodTypeAny>(
    method: 'POST' | 'PATCH' | 'PUT' | 'DELETE',
    path: string,
    schema: S,
    body?: unknown
): Promise<z.infer<S>> {
    const raw = await call('api_send', RawJson, { method, path, body })

    return parseOrThrow(path, schema, raw)
}

function parseOrThrow<S extends z.ZodTypeAny>(
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

    return parsed.data as z.infer<S>
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

    facets: (kind: ContentKindT) => get('/facets', FacetsResponseSchema, { kind }),

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
}
