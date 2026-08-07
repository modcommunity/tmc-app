import { z } from 'zod'

import { call } from '~/lib/ipc'
import {
    BrowseQuerySchema,
    BrowseResponseSchema,
    ContentDetailSchema,
    FacetsResponseSchema,
    InstallListResponse,
    InstallSchema,
    MeResponseSchema,
    SubscriptionSyncResponse,
    UserSettingsSchema,
    type BrowseQueryT,
    type ContentKindT,
    type InstallOptionsT,
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
