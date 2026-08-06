import z from 'zod'

/**
 * The contract for `/api/app/v1` — the app's whole view of TMC.
 *
 * **This file is a VERBATIM MIRROR of
 * `website-city/src/types/app-api/contract.ts`, which is the source of truth.**
 * Copy it across whenever the server side changes; do not edit it here
 * (`npm run contract:sync` does the copy). Every response is parsed through
 * these schemas before it reaches a component, so a drift shows up as a loud
 * validation error on the first request rather than as an `undefined` three
 * screens deep.
 *
 * Why a separate API at all, when tRPC already exists:
 *
 *   * **The website's routers return Prisma payloads.** Those are shaped by
 *     what a page renders and change whenever a page does. An installed app is
 *     not redeployed with the server, so its wire format has to be versioned
 *     and deliberately narrow — `/v1` means something here in a way it does not
 *     for a router the browser refetches on every deploy.
 *   * **Every content type is a different shape.** Assets, mods, servers and
 *     maps have separate browsers with separate filters and separate row
 *     payloads. The app draws one grid, so it wants one row type; normalising
 *     server-side is the only place that can be done once.
 *   * **Rust calls it too.** Latency probing needs a server's address without
 *     going through the React layer.
 */

export const APP_API_VERSION = 'v1'

// --------------------------------------------------------------- Envelope

/**
 * Every failure looks the same, and none of them describe internals.
 *
 * `code` is the stable thing clients branch on; `message` is for humans and may
 * be reworded at any time. `retryAfter` is present on 429 and on the device
 * poll's slow-down.
 */
export const ApiErrorSchema = z.object({
    ok: z.literal(false),
    code: z.string(),
    message: z.string(),
    retryAfter: z.number().int().nonnegative().optional(),
})

export type ApiErrorT = z.infer<typeof ApiErrorSchema>

export function ApiOk<S extends z.ZodTypeAny>(schema: S) {
    return z.object({ ok: z.literal(true), data: schema })
}

// ------------------------------------------------------------------- Auth

/** Self-reported client identity. Untrusted — a label for the approval screen. */
export const ClientInfoSchema = z.object({
    name: z.string().min(1).max(64),
    platform: z.enum([
        'windows',
        'macos',
        'linux',
        'android',
        'ios',
        'unknown',
    ]),
    version: z.string().min(1).max(32),
})

export type ClientInfoT = z.infer<typeof ClientInfoSchema>

export const DeviceStartRequest = z.object({
    client: ClientInfoSchema,
    /** PKCE S256 challenge, base64url. */
    codeChallenge: z.string().min(43).max(128),
})

export const DeviceStartResponse = z.object({
    deviceCode: z.string(),
    userCode: z.string(),
    /** Where to send the user when the code has to be typed. */
    verificationUri: z.string(),
    /** Same page with the code pre-filled — what the app actually opens. */
    verificationUriComplete: z.string(),
    expiresIn: z.number().int().positive(),
    interval: z.number().int().positive(),
})

export const DeviceTokenRequest = z.object({
    deviceCode: z.string().min(8).max(128),
    codeVerifier: z.string().min(43).max(128),
})

export const SessionUserSchema = z.object({
    id: z.string(),
    name: z.string().nullable(),
    username: z.string().nullable(),
    avatar: z.string().nullable(),
    role: z.string().nullable(),
})

export type SessionUserT = z.infer<typeof SessionUserSchema>

export const TokenResponse = z.object({
    accessToken: z.string(),
    refreshToken: z.string(),
    /** Seconds until `accessToken` stops working. */
    expiresIn: z.number().int().positive(),
    user: SessionUserSchema,
})

export type TokenResponseT = z.infer<typeof TokenResponse>

export const RefreshRequest = z.object({
    refreshToken: z.string().min(8).max(256),
})

/**
 * Poll outcomes, as their own `code` values on the error envelope.
 *
 * `authorization_pending` and `slow_down` are NOT failures in any sense the
 * user should see — they are the normal state of a flow that is waiting on a
 * human. Naming them keeps the app from having to guess from a status code.
 */
export const DEVICE_PENDING = 'authorization_pending'
export const DEVICE_SLOW_DOWN = 'slow_down'
export const DEVICE_DENIED = 'access_denied'
export const DEVICE_EXPIRED = 'expired_token'

// -------------------------------------------------------------- Content

export const ContentKindVals = [
    'asset',
    'mod',
    'server',
    'serverMap',
    'article',
    'community',
    'collection',
    'user',
] as const

export const ContentKindSchema = z.enum(ContentKindVals)
export type ContentKindT = (typeof ContentKindVals)[number]

/** Absolute URLs already resolved server-side — the app never builds a CDN path. */
export const ImageSetSchema = z.object({
    card: z.string().nullable(),
    banner: z.string().nullable(),
    icon: z.string().nullable(),
})

export const RefSchema = z.object({
    id: z.number().int(),
    name: z.string(),
    url: z.string().nullable(),
})

export const UserRefSchema = z.object({
    id: z.string(),
    name: z.string().nullable(),
    username: z.string().nullable(),
    avatar: z.string().nullable(),
})

export const StatsSchema = z.object({
    views: z.number().int().nonnegative(),
    downloads: z.number().int().nonnegative(),
    favorites: z.number().int().nonnegative(),
    comments: z.number().int().nonnegative(),
    reviews: z.number().int().nonnegative(),
    likes: z.number().int().nonnegative(),
    /** Mean review score 1–5, or null when nothing is scored. */
    rating: z.number().nullable(),
})

/**
 * The query protocols an app's servers can speak.
 *
 * Mirrors Prisma's `SpyQueryProtocols` exactly, which is the same list the
 * site's own scanners use. The app speaks these natively and queries servers
 * itself, so a player sees latency and player counts measured from THEIR
 * connection rather than from our scanner's — which is the whole reason the
 * app can do something a browser tab cannot.
 */
export const QueryProtocolVals = [
    'A2S',
    'MINECRAFT',
    'MINECRAFT_SLP',
    'QUAKE3',
    'DISCORD',
    'TEAMSPEAK3',
    'HYTALE_NITRADO',
    'FIVEM',
    'FROSTBITE',
    'GAMESPY1',
    'GAMESPY2',
    'GAMESPY3',
    'GAMESPY4',
    'GTA_NETWORK',
    'GTA_RAGE',
    'SAMP',
    'SCUM',
] as const

export const QueryProtocolSchema = z.enum(QueryProtocolVals)
export type QueryProtocolT = (typeof QueryProtocolVals)[number]

/**
 * Everything the app needs to query this game's servers itself.
 *
 * The port rules are the site's own scanner config. They are exposed rather
 * than reimplemented because getting the query port wrong is the single most
 * common reason a live server shows as dead — and the site has already worked
 * out the right answer per game.
 */
export const AppQueryConfigSchema = z.object({
    /** In preference order. The app uses the first it implements. */
    protocols: z.array(QueryProtocolSchema),
    /** `App.srvQueryTimeout`, milliseconds. */
    timeoutMs: z.number().int().positive(),
    /** `App.srvSwapGamePort` — query the game port itself. */
    swapGamePort: z.boolean(),
    /** `App.srvGamePortQueryOffset` — added to the game port otherwise. */
    portOffset: z.number().int(),
})

export type AppQueryConfigT = z.infer<typeof AppQueryConfigSchema>

/** Live server state. Present only on `kind === 'server'`. */
export const ServerInfoSchema = z.object({
    online: z.boolean(),
    /** Null when the owner has hidden the network details. */
    host: z.string().nullable(),
    port: z.number().int().nullable(),
    queryPort: z.number().int().nullable(),
    curUsers: z.number().int().nonnegative(),
    maxUsers: z.number().int().nonnegative(),
    bots: z.number().int().nonnegative(),
    password: z.boolean(),
    secure: z.boolean(),
    version: z.string().nullable(),
    gameMode: z.string().nullable(),
    map: z.string().nullable(),
    region: z.string().nullable(),
    country: z.string().nullable(),
    /** Deep link the app hands to the game, e.g. `steam://connect/…`. */
    connectUrl: z.string().nullable(),

    /**
     * How to query this server live. Null when the game declares no protocol,
     * or when the owner hid the network details — in which case there is
     * nothing to query and the app must not pretend otherwise.
     */
    query: AppQueryConfigSchema.nullable(),
})

export type ServerInfoT = z.infer<typeof ServerInfoSchema>

/** One row in any browser. Same shape for every kind — that is the point. */
export const ContentSummarySchema = z.object({
    kind: ContentKindSchema,
    /** String because users are cuids and everything else is an int. */
    id: z.string(),
    url: z.string().nullable(),
    name: z.string(),
    description: z.string().nullable(),

    images: ImageSetSchema,

    app: RefSchema.nullable(),
    owner: UserRefSchema.nullable(),
    community: RefSchema.nullable(),

    categories: z.array(RefSchema),
    tags: z.array(RefSchema),

    createdAt: z.string(),
    updatedAt: z.string().nullable(),

    nsfw: z.boolean(),
    archived: z.boolean(),
    isOfficial: z.boolean(),

    stats: StatsSchema,

    server: ServerInfoSchema.nullable(),

    /** Web permalink — "Open on the website" and share sheets. */
    webUrl: z.string(),
})

export type ContentSummaryT = z.infer<typeof ContentSummarySchema>

export const ReleaseFileSchema = z.object({
    id: z.string(),
    title: z.string().nullable(),
    size: z.number().int().nonnegative().nullable(),
    /** Absolute download URL. Authenticated downloads still resolve here. */
    url: z.string(),
})

export const ReleaseSchema = z.object({
    id: z.number().int(),
    version: z.string().nullable(),
    createdAt: z.string(),
    files: z.array(ReleaseFileSchema),
})

export type ReleaseT = z.infer<typeof ReleaseSchema>

export const MediaSchema = z.object({
    id: z.string(),
    type: z.string(),
    title: z.string().nullable(),
    url: z.string().nullable(),
})

export const LinkSchema = z.object({
    id: z.number().int(),
    name: z.string().nullable(),
    url: z.string(),
    /** `isGitSrc` forges vs everything else, so the app can badge them. */
    isSource: z.boolean(),
})

/** A single item's full page. `summary` is byte-identical to its browser row. */
export const ContentDetailSchema = z.object({
    summary: ContentSummarySchema,
    /** Long-form body, markdown as authored. Rendered client-side. */
    content: z.string().nullable(),
    /** Server rules / usage terms, when the type has them. */
    rules: z.string().nullable(),
    releases: z.array(ReleaseSchema),
    media: z.array(MediaSchema),
    links: z.array(LinkSchema),
    /** Item ids this one requires, resolved to summaries where possible. */
    dependencies: z.array(ContentSummarySchema),
})

export type ContentDetailT = z.infer<typeof ContentDetailSchema>

// -------------------------------------------------------------- Browsing

export const BrowseSortVals = [
    'createdAt',
    'lastEdit',
    'name',
    'views',
    'downloads',
    'rating',
    'favorites',
    /** Servers only; ignored elsewhere rather than rejected. */
    'players',
] as const

export const BrowseSortSchema = z.enum(BrowseSortVals)
export type BrowseSortT = (typeof BrowseSortVals)[number]

export const TimeRangeVals = ['all', '24h', '7d', '30d'] as const
export const TimeRangeSchema = z.enum(TimeRangeVals)

/**
 * A boolean that survives a query string.
 *
 * `z.coerce.boolean()` is the trap here: it is `Boolean(value)`, so the string
 * `"false"` is `true`. These flags are all opt-IN, so anything that is not an
 * affirmative reads as false.
 */
export const QueryBool = z
    .union([z.boolean(), z.string()])
    .transform((v) => v === true || v === 'true' || v === '1')

/** An integer that may arrive as a string from a query parameter. */
const QueryInt = z.coerce.number().int()

export const BrowseQuerySchema = z.object({
    kind: ContentKindSchema,
    /*
     * `coerce` on the string fields too. They are already strings on the wire,
     * but a JSON caller (and the app's own client, which validates before
     * sending) may hand over a number for `cursor`, and rejecting that is a
     * distinction without a difference.
     */
    search: z.coerce.string().max(128).optional(),

    apps: z.array(QueryInt).max(50).optional(),
    categories: z.array(QueryInt).max(50).optional(),
    tags: z.array(QueryInt).max(50).optional(),
    communityId: QueryInt.optional(),
    ownerId: z.coerce.string().max(64).optional(),

    nsfw: QueryBool.optional(),
    archived: QueryBool.optional(),
    /** Servers only. */
    onlineOnly: QueryBool.optional(),

    sort: BrowseSortSchema.default('createdAt'),
    sortDir: z.enum(['asc', 'desc']).default('desc'),
    timeRange: TimeRangeSchema.default('all'),

    /**
     * Cursor paging only — the app's grids are infinite, never numbered.
     *
     * Coerced because every kind but `user` has an integer id, so the cursor
     * the server handed out comes back looking like a number.
     */
    cursor: z.coerce.string().nullish(),
    limit: QueryInt.min(1).max(50).default(30),
})

export type BrowseQueryT = z.input<typeof BrowseQuerySchema>

export const BrowseResponseSchema = z.object({
    items: z.array(ContentSummarySchema),
    /** Pass back as `cursor`. Null means the end. */
    nextCursor: z.string().nullable(),
    /** Total matching the filters, when the backend can produce one cheaply. */
    total: z.number().int().nonnegative().nullable(),
})

export type BrowseResponseT = z.infer<typeof BrowseResponseSchema>

/** Sidebar facets, so the app's filters are populated from real data. */
export const FacetsResponseSchema = z.object({
    apps: z.array(RefSchema.extend({ count: z.number().int() })),
    categories: z.array(
        RefSchema.extend({
            count: z.number().int(),
            parentId: z.number().int().nullable(),
        })
    ),
})

// -------------------------------------------------------------- Settings

/**
 * The subset of `UserSettings` the app both reads and writes.
 *
 * Deliberately not the whole row: settings that only mean something in a
 * browser (chat auto-open) or that the app has no UI for would become dead
 * fields the app is nonetheless responsible for round-tripping correctly.
 * Everything here is a plain boolean or a short string, so a PATCH is a
 * partial of exactly this shape.
 */
export const UserSettingsSchema = z.object({
    timezone: z.string().max(64),
    locale: z.string().max(16),

    emailNotifications: z.boolean(),
    pushNotifications: z.boolean(),

    notifySystem: z.boolean(),
    notifyComments: z.boolean(),
    notifyReviews: z.boolean(),
    notifyReleases: z.boolean(),
    notifyMentions: z.boolean(),
    notifyModeration: z.boolean(),
    notifyCredits: z.boolean(),
    notifyFriends: z.boolean(),
    notifyFollowers: z.boolean(),
    notifyMessages: z.boolean(),
    notifyStatus: z.boolean(),
    notifyParties: z.boolean(),
    notifyGroups: z.boolean(),
})

export type UserSettingsT = z.infer<typeof UserSettingsSchema>

export const UserSettingsPatchSchema = UserSettingsSchema.partial()

// ------------------------------------------------------------------ /me

export const MeResponseSchema = z.object({
    user: SessionUserSchema,
    settings: UserSettingsSchema,
    /** Server clock, so the app can detect a badly skewed device. */
    serverTime: z.number().int(),
})

export type MeResponseT = z.infer<typeof MeResponseSchema>
