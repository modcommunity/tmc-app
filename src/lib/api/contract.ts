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
    platform: z.enum(['windows', 'macos', 'linux', 'android', 'ios', 'unknown']),
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
 * Redeem a web-player handoff code for a game session credential.
 *
 * The other half of `GameBootDescriptorT.auth` — see `~/types/play/auth.ts` for
 * why the descriptor carries a code rather than a token. Unauthenticated by
 * necessity and by design: the code IS the credential, it is single-use, it
 * lives sixty seconds, and it was minted for the person holding it.
 *
 * The response is the same {@link TokenResponse} the device flow returns, so a
 * client that already knows how to hold a TMC session does not learn a second
 * shape. What differs is the REACH of what it gets: an `AppToken` of kind
 * `GAME`, bound to the app it was minted for, refused by every route that has
 * not opted in.
 */
export const PlayHandoffRequest = z.object({
    code: z.string().min(8).max(256),
    /**
     * Self-reported, and shown to the member on Account → Devices exactly like
     * a device's is. UNTRUSTED — a label, never an authorisation input.
     */
    client: ClientInfoSchema.optional(),
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

/**
 * A game, with its artwork.
 *
 * A plain [`RefSchema`] plus `icon`, because the app draws one flat grid across
 * every game in the catalogue — a row's game is not implied by the surrounding
 * chrome the way it is on a site built around one chosen game.
 *
 * `icon` is optional with a null default so a build talking to a server that
 * predates it renders text-only rather than failing to parse the whole browse
 * response.
 */
export const AppRefSchema = RefSchema.extend({
    icon: z.string().nullable().default(null),
})

export type AppRefT = z.infer<typeof AppRefSchema>

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
 * A subset of Prisma's `SpyQueryProtocols`, which is the same list the site's
 * own scanners use. The app speaks these natively and queries servers itself,
 * so a player sees latency and player counts measured from THEIR connection
 * rather than from our scanner's — which is the whole reason the app can do
 * something a browser tab cannot.
 *
 * `PALWORLD_REST` is deliberately NOT here. It is the one protocol that needs a
 * credential — the server's admin password — and the app has none and must
 * never be handed one. `buildQueryConfig` drops protocols this list doesn't
 * know, so a Palworld app configured with both simply reaches the app as `A2S`.
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
    /**
     * Rolling average population, as the website's server table shows it.
     *
     * The number that separates "quiet right now" from "always empty", which
     * `curUsers` alone cannot: a server at 0/32 on a Tuesday morning and one
     * that has never had a player look identical until you see this.
     */
    avgUsers: z.number().int().nonnegative().default(0),
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

    app: AppRefSchema.nullable(),
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

/**
 * How one item relates to another. Mirrors Prisma's `DependencyType`.
 *
 * `Conflict` is the reason this is on the wire at all: collapsing every edge
 * into "dependencies" turns "these two must not be installed together" into
 * "install this one too", which is the exact opposite of what the author wrote.
 */
export const DependencyRelationVals = [
    'Required',
    'Optional',
    'Recommended',
    'Conflict',
] as const

export const DependencyRelationSchema = z.enum(DependencyRelationVals)
export type DependencyRelationT = (typeof DependencyRelationVals)[number]

/**
 * One dependency edge, in the narrow shape the app acts on.
 *
 * A name, an id, something to draw, and the edge type — not a full
 * `ContentSummary`. The app renders these as a list and uses them to answer one
 * question (*what else do I need, and is anything going to fight?*), so a card
 * row would be three times the payload for a screen that renders none of it.
 * Tapping one navigates by `kind` and `id`, which fetches the real row.
 */
export const ContentDependencySchema = z.object({
    relation: DependencyRelationSchema,
    /** The author's note about this edge, markdown. */
    note: z.string().nullable(),
    kind: ContentKindSchema,
    id: z.number().int(),
    name: z.string(),
    icon: z.string().nullable(),
})

export type ContentDependencyT = z.infer<typeof ContentDependencySchema>

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

    /**
     * What this item needs, recommends, and cannot live beside.
     *
     * The element type CHANGED from `ContentSummary` to
     * {@link ContentDependencySchema}. That is a wire break in principle and
     * not one in practice: the field was hardcoded to `[]` from the day it was
     * declared until the day this shipped, so no build has ever rendered an
     * element of it. Carrying a second, parallel field forever to avoid a
     * change nobody can observe would be the worse trade.
     */
    dependencies: z.array(ContentDependencySchema),
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
    'reviews',
    'favorites',

    /*
     * Server-only sorts, named exactly as `ServerSortVals` names them.
     *
     * `curUsers` replaces the app's old `players`, which was a name this
     * contract invented. Every other sort here already matched the website's
     * spelling, so the one that did not was the one nobody could grep for.
     * A sort a kind does not support is IGNORED rather than rejected — the app
     * changes kind without resetting the sort, and a 400 mid-browse would be a
     * worse answer than "newest first".
     */
    'curUsers',
    'maxUsers',
    'avgUsers',
    'bots',
    'map',
    'lastOnline',
    'lastScanned',

    /**
     * @deprecated The old app-only spelling of `curUsers`.
     *
     * Kept in the enum because an installed build is not redeployed with the
     * server and will go on sending it. Removing it would 400 every server
     * browse from every shipped copy of the app, which is the exact failure a
     * versioned wire format exists to prevent. Ordering treats it as `curUsers`.
     */
    'players',
] as const

export const BrowseSortSchema = z.enum(BrowseSortVals)
export type BrowseSortT = (typeof BrowseSortVals)[number]

/** Runtime environment for mods and assets. Mirrors `EnvironmentVals`. */
export const BrowseEnvironmentVals = ['ALL', 'SERVER', 'CLIENT'] as const
export const BrowseEnvironmentSchema = z.enum(BrowseEnvironmentVals)
export type BrowseEnvironmentT = (typeof BrowseEnvironmentVals)[number]

/** A server's operating system. Mirrors `ServerOsVals`. */
export const ServerOsVals = ['WINDOWS', 'LINUX', 'MAC'] as const
export const ServerOsSchema = z.enum(ServerOsVals)
export type ServerOsT = (typeof ServerOsVals)[number]

/**
 * A community's stated minimum age. Mirrors `CommunityAgeVals`.
 *
 * A CLAIM by the community's owner, never an enforced gate — nothing verifies
 * anybody's age. Any surface rendering it must read as "this community says".
 */
export const CommunityAgeVals = [
    'NONE',
    'ADULT_18',
    'ADULT_21',
    'ADULT_25',
] as const
export const CommunityAgeSchema = z.enum(CommunityAgeVals)
export type CommunityAgeT = (typeof CommunityAgeVals)[number]

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

    /**
     * AND across tags by default; `tagsOr: true` switches to ANY.
     *
     * The website's browsers offer both. The app's chips read as "all of
     * these", which is why AND stays the default here.
     */
    tagsOr: QueryBool.optional(),

    /** Mods and assets. `ALL`/omitted filters nothing — see `EnvironmentWhere`. */
    environment: BrowseEnvironmentSchema.optional(),

    // ------------------------------------------------------------- Servers
    //
    // Everything below mirrors `ServerBrowserPublicGetsInput`, which is the
    // exact filter set the website's own server browser exposes. All optional,
    // all ignored by the kinds that cannot express them — a filter panel that
    // survives a kind switch is worth more than a 400.

    /** Servers only. */
    onlineOnly: QueryBool.optional(),
    /** Substring match on the server's current map name. */
    mapName: z.coerce.string().max(128).optional(),
    /** Country ids, from `/facets`. */
    countries: z.array(QueryInt).max(50).optional(),
    os: ServerOsSchema.optional(),
    password: QueryBool.optional(),
    secure: QueryBool.optional(),
    isOfficial: QueryBool.optional(),

    hideEmpty: QueryBool.optional(),
    hideFull: QueryBool.optional(),

    /** Current population (`Server.curUsers`). */
    minUsers: QueryInt.min(0).optional(),
    maxUsers: QueryInt.min(0).optional(),
    /** Capacity (`Server.maxUsers`). Named "slots" to keep the two apart. */
    minSlots: QueryInt.min(0).optional(),
    maxSlots: QueryInt.min(0).optional(),

    /**
     * Only servers seen online inside the inactivity-removal window.
     *
     * NOT "has ever been online" — that only excluded servers a nightly job had
     * already cleared, which is why the website's own field carries the same
     * warning.
     */
    wasOnline: QueryBool.optional(),

    /**
     * Restrict to servers whose COMMUNITY states one of these minimum ages.
     *
     * Include-only, never a mute list: an empty or absent array applies no
     * restriction. Nothing here verifies anybody's age — it honours what a
     * community says about itself.
     */
    adultAges: z.array(CommunityAgeSchema).max(8).optional(),

    /** The signed-in user's own items. Resolved server-side from the token. */
    mine: QueryBool.optional(),

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
    apps: z.array(
        RefSchema.extend({
            count: z.number().int(),
            /**
             * The game's own artwork, absolute.
             *
             * Optional with a null default so an app talking to a server that
             * predates it renders a text-only picker rather than failing to
             * parse the whole facets response.
             */
            icon: z.string().nullable().default(null),
        })
    ),
    categories: z.array(
        RefSchema.extend({
            count: z.number().int(),
            parentId: z.number().int().nullable(),
        })
    ),
    /**
     * Countries that actually host servers, for the server browser's region
     * filter. Empty for every other kind — nothing else has a location.
     *
     * Counted from the servers themselves rather than listing the country
     * table: the app would otherwise offer ~200 countries, most of which select
     * nothing, and a filter that can only ever return an empty list is worse
     * than no filter.
     */
    countries: z.array(RefSchema.extend({ count: z.number().int() })).default([]),
})

export type FacetsResponseT = z.infer<typeof FacetsResponseSchema>

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

// ----------------------------------------------------------- Subscriptions

/**
 * What can be subscribed to — "keep this on my devices".
 *
 * Three of the eight content kinds, and the closed list is load-bearing: a
 * server is not something you install, an article is not something a device
 * holds a copy of. Mirrors `SubKindVals` in `~/types/subscription/type`.
 */
export const SubKindVals = ['asset', 'mod', 'collection'] as const

export const SubKindSchema = z.enum(SubKindVals)
export type SubKindT = (typeof SubKindVals)[number]

/**
 * One subscribed item, as the app's library renders it and its installer acts
 * on it.
 *
 * `file` is the thing the app actually downloads. It is nullable rather than
 * required because a subscription outlives the release it was made against: an
 * item whose only release is later hidden still has a live subscription, and
 * the app has to render it as "nothing to install yet" rather than fail to
 * parse the whole list.
 */
export const SubscriptionFileSchema = z.object({
    id: z.string(),
    /** Absolute download URL. Bearer-authenticated downloads resolve here. */
    url: z.string(),
    size: z.number().int().nonnegative().nullable(),
    /**
     * Lower-case hex SHA-256 of the file, when we have one.
     *
     * The app verifies against this after downloading and DELETES the file on a
     * mismatch — see the plugin sandbox's `download` step. Null means "we do not
     * have a digest", which the app treats as "install it but say so", not as
     * "verified".
     */
    sha256: z.string().nullable(),
    /** Original filename, so the installer can honour an extension rule. */
    name: z.string().nullable(),
})

export type SubscriptionFileT = z.infer<typeof SubscriptionFileSchema>

export const SubscriptionSchema = z.object({
    id: z.string(),
    kind: SubKindSchema,
    /** The content item's own id. */
    itemId: z.number().int(),
    name: z.string(),
    description: z.string().nullable(),
    image: z.string().nullable(),
    /** The item's page on the website — "Open on the website". */
    webUrl: z.string(),

    /**
     * The app this item installs into, and the URL SEGMENT its plugins live
     * under (`plugins/app/<slug>/…`). Null for a collection, which is not
     * app-scoped in the schema.
     */
    app: z
        .object({
            id: z.number().int(),
            name: z.string(),
            slug: z.string().nullable(),
        })
        .nullable(),

    createdAt: z.string(),
    updatedAt: z.string(),

    autoUpdate: z.boolean(),
    notifyUpdates: z.boolean(),
    paused: z.boolean(),

    /** Set when a collection subscription created this row. */
    viaCollectionId: z.number().int().nullable(),

    /**
     * Still installable? False once the item's team opts out or its game loses
     * app support. The app leaves such an item alone rather than uninstalling
     * it — a user who has it working should not lose it because of a flag.
     */
    installable: z.boolean(),

    release: z
        .object({
            id: z.number().int(),
            version: z.string().nullable(),
            createdAt: z.string(),
            file: SubscriptionFileSchema.nullable(),
        })
        .nullable(),
})

export type SubscriptionT = z.infer<typeof SubscriptionSchema>

/**
 * The sync response.
 *
 * `revision` is a monotonic watermark the app sends back on its next poll. The
 * server answers with everything changed since — which is what lets a device
 * poll every minute without transferring the whole library each time, and what
 * makes a subscription created in the BROWSER show up on the desktop within one
 * poll. `full` tells the app the answer is a complete list rather than a delta,
 * so it can drop anything it holds that is not in it.
 */
export const SubscriptionSyncResponse = z.object({
    items: z.array(SubscriptionSchema),
    /** Ids removed since `since`. Empty on a full sync. */
    removed: z.array(z.string()),
    revision: z.string(),
    full: z.boolean(),
})

export type SubscriptionSyncResponseT = z.infer<typeof SubscriptionSyncResponse>

export const SubscriptionQuerySchema = z.object({
    /**
     * The `revision` from the previous response. Omitted means "give me
     * everything" — which is what a fresh install and a re-login both do.
     */
    since: z.coerce.string().max(64).optional(),
    limit: z.coerce.number().int().min(1).max(200).default(200),
})

export const SubscriptionWriteRequest = z.object({
    kind: SubKindSchema,
    itemId: z.coerce.number().int().positive(),
    /** Absent flips it; present forces a state. */
    subscribed: z.boolean().optional(),
})

export const SubscriptionPrefsRequest = z.object({
    kind: SubKindSchema,
    itemId: z.coerce.number().int().positive(),
    autoUpdate: z.boolean().optional(),
    notifyUpdates: z.boolean().optional(),
    paused: z.boolean().optional(),
})

// ----------------------------------------------------------------- Installs

export const GraphicsPresetVals = [
    'default',
    'low',
    'medium',
    'high',
    'ultra',
] as const

export const WindowModeVals = [
    'default',
    'windowed',
    'borderless',
    'fullscreen',
] as const

/**
 * An install's declarative launch options.
 *
 * Every field optional, every one with a "leave it alone" meaning: the
 * overwhelmingly common install overrides nothing, and a launcher that silently
 * forces 1920×1080 on somebody who never asked is worse than one with no
 * settings at all. What each key MEANS for a given game is decided by that
 * game's launch plugin, not here.
 */
export const InstallOptionsSchema = z.object({
    graphics: z.enum(GraphicsPresetVals).optional(),
    windowMode: z.enum(WindowModeVals).optional(),
    width: z.number().int().min(320).max(16384).optional(),
    height: z.number().int().min(240).max(16384).optional(),
    monitor: z.number().int().min(0).max(15).optional(),
    fpsLimit: z.number().int().min(0).max(1000).optional(),
    vsync: z.boolean().optional(),
    memoryMb: z.number().int().min(256).max(65536).optional(),
    hideOnLaunch: z.boolean().optional(),
    confirmUpdates: z.boolean().optional(),
})

export type InstallOptionsT = z.infer<typeof InstallOptionsSchema>

export const InstallItemSchema = z.object({
    id: z.number().int(),
    kind: SubKindSchema,
    itemId: z.number().int(),
    name: z.string(),
    webUrl: z.string(),
    image: z.string().nullable(),
    enabled: z.boolean(),
    order: z.number().int(),
    /** False when the user is not subscribed — the app must not materialise it. */
    subscribed: z.boolean(),
})

/**
 * How the app puts a sandbox's files in front of the game.
 *
 * `direct` copies into the game folder (and backs up whatever it displaced);
 * the two link strategies leave the game folder made of pointers; `usvfs` is
 * Mod Organizer's runtime API hooking, which the app declares and does not yet
 * implement.
 *
 * The server stores this as a STRING and validates it against this list, but a
 * value it does not recognise is not fatal to the app — it falls back to its
 * own default. That is what lets a new strategy ship in the app before it ships
 * here.
 */
export const DeployStrategyVals = [
    'direct',
    'hardlink',
    'symlink',
    'usvfs',
] as const

export const DeployStrategySchema = z.enum(DeployStrategyVals)
export type DeployStrategyT = (typeof DeployStrategyVals)[number]

/**
 * Which side of a game a sandbox is for.
 *
 * Not decoration: it decides which mods are offered for it and which of the
 * game's own options are shown. A dedicated server and a player's game are the
 * same files arranged differently, and a manager that cannot tell them apart
 * offers everybody both halves of every list.
 *
 * `shared` accepts both, which is what the website's own `ALL` environment
 * means on a mod.
 */
export const SandboxEnvVals = ['client', 'server', 'shared'] as const

export const SandboxEnvSchema = z.enum(SandboxEnvVals)
export type SandboxEnvT = (typeof SandboxEnvVals)[number]

export const InstallSchema = z.object({
    id: z.number().int(),
    appId: z.number().int(),
    app: z.object({
        id: z.number().int(),
        name: z.string(),
        slug: z.string().nullable(),
        icon: z.string().nullable(),
    }),
    name: z.string(),
    description: z.string().nullable(),
    gameVersion: z.string().nullable(),
    loader: z.string().nullable(),
    isDefault: z.boolean(),
    /**
     * Defaulted rather than required, so an app talking to a server that
     * predates these fields gets a working sandbox instead of a parse error.
     */
    strategy: DeployStrategySchema.default('hardlink'),
    environment: SandboxEnvSchema.default('client'),
    launchArgs: z.array(z.string()),
    launchEnv: z.record(z.string(), z.string()),
    options: InstallOptionsSchema,
    createdAt: z.string(),
    updatedAt: z.string(),
    lastPlayedAt: z.string().nullable(),
    playSeconds: z.number().int().nonnegative(),
    items: z.array(InstallItemSchema),
})

export type InstallT = z.infer<typeof InstallSchema>

export const InstallListResponse = z.object({
    installs: z.array(InstallSchema),
})

export const InstallLaunchSchema = z.object({
    args: z.array(z.string().min(1).max(256)).max(64).optional(),
    env: z.record(z.string().max(64), z.string().max(512)).optional(),
    options: InstallOptionsSchema.optional(),
})

export const InstallCreateRequest = z.object({
    appId: z.coerce.number().int().positive(),
    name: z.string().min(1).max(64),
    description: z.string().max(500).optional(),
    gameVersion: z.string().max(64).optional(),
    loader: z.string().max(64).optional(),
    isDefault: z.boolean().optional(),
    strategy: DeployStrategySchema.optional(),
    environment: SandboxEnvSchema.optional(),
    launch: InstallLaunchSchema.optional(),
})

export const InstallUpdateRequest = z.object({
    id: z.coerce.number().int().positive(),
    name: z.string().min(1).max(64).optional(),
    description: z.string().max(500).nullable().optional(),
    gameVersion: z.string().max(64).nullable().optional(),
    loader: z.string().max(64).nullable().optional(),
    isDefault: z.boolean().optional(),
    strategy: DeployStrategySchema.optional(),
    environment: SandboxEnvSchema.optional(),
    launch: InstallLaunchSchema.optional(),
    /**
     * Playtime the app is reporting for this install, in seconds since the last
     * report. Clamped server-side — it is a number a client chose.
     */
    playedSeconds: z.number().int().min(0).max(86400).optional(),
})

export const InstallDeleteRequest = z.object({
    id: z.coerce.number().int().positive(),
})

export const InstallItemRequest = z.object({
    installId: z.coerce.number().int().positive(),
    kind: SubKindSchema,
    itemId: z.coerce.number().int().positive(),
    /** Present on a patch; absent on add/remove. */
    enabled: z.boolean().optional(),
    order: z.number().int().min(0).max(10000).optional(),
})

// --------------------------------------------------------- Device downloads

/**
 * What one device is downloading, as it reports it.
 *
 * The queue itself lives on the device and is the authority for everything —
 * this is a **snapshot for display**, so a user on their phone can see whether
 * the modpack their desktop started has finished. Nothing on the server side
 * ever drives a download.
 *
 * Note what a row does NOT carry: no URL, no filesystem path, no device
 * identifier beyond the user's own label for the machine. A path would put
 * somebody's username and drive layout in a row that is one authorisation bug
 * away from public, in exchange for nothing anybody would read.
 */
export const DeviceDownloadItemSchema = z.object({
    id: z.string().max(128),
    label: z.string().max(200),
    status: z.enum(['queued', 'running', 'paused', 'done', 'failed', 'cancelled']),
    done: z.number().int().nonnegative(),
    total: z.number().int().nonnegative().nullable(),
    speedBps: z.number().int().nonnegative(),
})

export type DeviceDownloadItemT = z.infer<typeof DeviceDownloadItemSchema>

/** Cap on the rows one report may carry. A modpack is hundreds; a list is not. */
export const MAX_REPORTED_DOWNLOADS = 25

export const DeviceDownloadReportRequest = z.object({
    active: z.number().int().min(0).max(100000),
    queued: z.number().int().min(0).max(100000),
    paused: z.number().int().min(0).max(100000),
    failed: z.number().int().min(0).max(100000),
    finished: z.number().int().min(0).max(100000),
    speedBps: z.number().int().min(0),
    remainingBytes: z.number().int().min(0),
    /**
     * The in-flight rows, newest first, capped. The app sends the ones worth
     * showing rather than the whole queue — the summary counters above are what
     * a device list renders, and the items are for the one device somebody
     * opened.
     */
    items: z
        .array(DeviceDownloadItemSchema)
        .max(MAX_REPORTED_DOWNLOADS)
        .default([]),
})

export const DeviceDownloadSchema = z.object({
    deviceName: z.string(),
    /** Whether this is the device asking. */
    isThisDevice: z.boolean(),
    active: z.number().int(),
    queued: z.number().int(),
    paused: z.number().int(),
    failed: z.number().int(),
    finished: z.number().int(),
    speedBps: z.number().int(),
    remainingBytes: z.number().int(),
    items: z.array(DeviceDownloadItemSchema),
    updatedAt: z.string(),
})

export const DeviceDownloadListResponse = z.object({
    devices: z.array(DeviceDownloadSchema),
})

export type DeviceDownloadT = z.infer<typeof DeviceDownloadSchema>

// ----------------------------------------------------------------- Reviews

/**
 * One review, as the app renders it.
 *
 * Narrower than the website's own row on purpose. The site shows five separate
 * sub-scores for some content types; the app shows one number and the text,
 * because a phone-sized card with six ratings on it is a card nobody reads. The
 * sub-scores stay on the website, where there is room for them.
 */
export const ReviewSchema = z.object({
    id: z.number().int(),
    owner: UserRefSchema.nullable(),
    /** 1–5, or null for a review that is only text. */
    rating: z.number().int().min(1).max(5).nullable(),
    content: z.string().nullable(),
    createdAt: z.string(),
    lastEdit: z.string().nullable(),
    /** Net helpful score, as the site counts it. */
    score: z.number().int(),
    /** True when this review is the caller's own. */
    mine: z.boolean().default(false),
    /**
     * The caller's own helpful vote, or null.
     *
     * Sent with the list rather than fetched per row: a page of twenty reviews
     * would otherwise be twenty-one requests, and the button has to render in
     * its correct state on first paint or it flickers.
     */
    myVote: z.boolean().nullable().default(null),
})

export type ReviewT = z.infer<typeof ReviewSchema>

export const ReviewListResponse = z.object({
    reviews: z.array(ReviewSchema),
    nextCursor: z.string().nullable(),
    /** How many of each score, so the app can draw the distribution bars. */
    breakdown: z.object({
        one: z.number().int().nonnegative(),
        two: z.number().int().nonnegative(),
        three: z.number().int().nonnegative(),
        four: z.number().int().nonnegative(),
        five: z.number().int().nonnegative(),
    }),
    /** Mean, or null when nothing is scored. */
    average: z.number().nullable(),
    total: z.number().int().nonnegative(),
})

/**
 * Leaving or editing a review.
 *
 * One per person per item, and a repeat call UPDATES — the model's
 * `@@unique([ownerId, modId])` and its siblings make a second insert an error
 * rather than a second review, so a create-only client would fail on every
 * edit.
 *
 * `rating` and `content` are both optional and at least one must be present:
 * the website allows a score with no words and words with no score, and
 * refusing either here would make the app stricter than the site for no reason.
 */
export const ReviewWriteRequest = z
    .object({
        kind: ContentKindSchema,
        id: z.coerce.number().int().positive(),
        rating: z.number().int().min(1).max(5).nullable().optional(),
        /**
         * Markdown. The operator's configured length limit is enforced
         * server-side by `AssertTextLimits`; this is only the hard ceiling.
         */
        content: z.string().max(20000).nullable().optional(),
    })
    .refine(
        (input) =>
            (input.rating ?? null) !== null ||
            (input.content ?? '').trim().length > 0,
        { message: 'Give it a score, some words, or both.' }
    )

export const ReviewWriteResponse = z.object({
    saved: z.literal(true),
    /** False for a first review, true for an edit. */
    edited: z.boolean(),
})

export const ReviewDeleteRequest = z.object({
    id: z.coerce.number().int().positive(),
})

/**
 * Marking a review helpful, or taking it back.
 *
 * `null` removes the vote. A helpful button with no way to un-press it is one
 * people press by accident once and resent forever.
 */
export const ReviewVoteRequest = z.object({
    id: z.coerce.number().int().positive(),
    positive: z.boolean().nullable(),
})

export const ReviewVoteResponse = z.object({
    voted: z.boolean().nullable(),
})

export const ReviewQuerySchema = z.object({
    kind: ContentKindSchema,
    id: z.coerce.number().int().positive(),
    sort: z.enum(['recent', 'helpful', 'rating']).default('recent'),
    cursor: z.coerce.string().nullish(),
    limit: z.coerce.number().int().min(1).max(50).default(20),
})

// ------------------------------------------------------------------ Reports

/**
 * Reporting something from inside the app.
 *
 * Deliberately the same shape the website's own form submits, with the same
 * `Report` row behind it — a report made in the app has to land in the same
 * moderation queue, or the app becomes a way to file something nobody sees.
 *
 * There is no "reason" enum beyond the website's own two types. A free-text
 * body is what moderators actually read, and a longer enum only ever produces
 * arguments about which bucket something belongs in.
 */
export const ReportKindVals = ['SPAM', 'OTHER'] as const

export const ReportCreateRequest = z.object({
    kind: ContentKindSchema,
    id: z.coerce.number().int().positive(),
    type: z.enum(ReportKindVals).default('OTHER'),
    title: z.string().min(3).max(120),
    content: z.string().min(10).max(4000),
})

export const ReportCreateResponse = z.object({
    reported: z.literal(true),
})

// ------------------------------------------------------------------ Version

/**
 * What `GET /api/app/v1/version` answers.
 *
 * A NOTICE, not an updater. There is no artifact URL, no signature and no
 * checksum here because the app does not update itself — it compares its own
 * version against `latest` and offers to open `download` in the user's real
 * browser. A self-updater needs a signing key held by whoever cuts releases and
 * a manifest this would have to serve; shipping the client half against neither
 * would be a feature that names a capability it does not have.
 *
 * Both fields are nullable and both mean "say nothing". Until a release is
 * published there is nothing to point anybody at, and an app told about a
 * version that does not exist sends people to a 404 — which reads as the app
 * being broken rather than as a field nobody filled in.
 */
export const AppVersionResponse = z.object({
    /** A version string, compared against the running build's. */
    latest: z.string().nullable(),
    /** Absolute https, or null. Where a user goes to get it. */
    download: z.string().nullable(),
})

export type AppVersionResponseT = z.infer<typeof AppVersionResponse>

/* ========================================================================== */
/*  Apps, and playing them                                                    */
/* ========================================================================== */

/**
 * THE APP CATALOGUE, AND WHAT THE APP MAY DO WITH IT.
 *
 * Everything below backs two screens the website has no equivalent of. The site
 * is built around ONE chosen game — its chrome names it, its browser is scoped
 * to it — so it has never needed to answer "what games are there, and which of
 * them can I actually start right now". The app opens on a flat list across the
 * whole catalogue and that IS its front door, so the question is unavoidable.
 *
 * Two endpoints:
 *
 *   * **`GET /apps`** — the catalogue. Every published app, with its artwork,
 *     what it is, how deeply it is wired into TMC, and — when it is playable —
 *     the launch vocabulary below.
 *   * **`POST /play/launch`** — resolve one launch, server-side. The app never
 *     decides whether something can be started by reading columns; it asks, and
 *     a refusal is `null` rather than an error, exactly as `play.launch` does
 *     for the website's own player.
 *
 * The duplication with `~/types/play/*` is deliberate and matches the rule at
 * the top of this file: the contract imports nothing but zod, because it is
 * copied verbatim into a repository that has no `~/types/play` to import from.
 * Every enum here is checked against its source by a test.
 */

/** Mirrors `AppType` in `prisma/models/app.prisma`. */
export const AppTypeVals = [
    'GAME',
    'GAME_SINGLEPLAYER',
    'GAME_ENGINE',
    'VOIP',
    'OTHER',
] as const

export const AppTypeSchema = z.enum(AppTypeVals)
export type AppTypeT = (typeof AppTypeVals)[number]

/**
 * Mirrors `AppIntegrationType`. Capability CLAIMS, not switches.
 *
 * The app reads two of them. `TMC_APP_MANAGE` is what puts a game in the "you
 * can mod this here" half of the Apps tab; `TMC_APP_WEB` is half of the `app`
 * play mode. Nothing here unlocks anything on its own — the launch endpoint
 * re-derives every decision from the columns that actually gate it.
 */
export const AppIntegrationVals = [
    'TMC_APP_MANAGE',
    'TMC_APP_TRACK',
    'TMC_APP_SRV_TRACK',
    'TMC_APP_SRV_AUTH',
    'TMC_APP_WEB',
    'TMC_APP_SRV_MANAGE',
] as const

export const AppIntegrationSchema = z.enum(AppIntegrationVals)
export type AppIntegrationT = (typeof AppIntegrationVals)[number]

/**
 * How a game can be launched. Mirrors `PlayModeT` in `~/types/play/mode.ts`.
 *
 * `web` is a JavaScript loader the app runs in an isolated window; `app` is a
 * deep link the WEBSITE hands to an installed copy of this app. The second is
 * listed here for completeness — the app itself never follows one, because the
 * URI's destination is the process already reading it.
 *
 * A third mode exists only on the device and deliberately is not here:
 * launching a copy of the game that is installed on this machine. The server
 * cannot know whether that is possible, so it is not the server's to declare.
 */
export const PlayModeVals = ['web', 'app'] as const
export const PlayModeSchema = z.enum(PlayModeVals)
export type PlayModeT = (typeof PlayModeVals)[number]

/** Mirrors `PlayOptionKindT`. */
export const PlayOptionKindVals = ['bool', 'select', 'int', 'text'] as const
export const PlayOptionKindSchema = z.enum(PlayOptionKindVals)

/**
 * One launch option an app declares.
 *
 * A FORM DESCRIPTION and nothing more — see `~/types/play/option.ts` for why it
 * cannot express a condition or a computation. The same reasoning as
 * `sandbox.json`'s `options`: a settings schema that can do those is a program,
 * and neither the site nor the app is willing to evaluate somebody else's.
 */
export const PlayOptionSchema = z.discriminatedUnion('kind', [
    z.object({
        key: z.string(),
        label: z.string(),
        help: z.string().optional(),
        modes: z.array(PlayModeSchema).optional(),
        kind: z.literal('bool'),
        def: z.boolean(),
    }),
    z.object({
        key: z.string(),
        label: z.string(),
        help: z.string().optional(),
        modes: z.array(PlayModeSchema).optional(),
        kind: z.literal('select'),
        def: z.string(),
        choices: z.array(z.object({ value: z.string(), label: z.string() })),
    }),
    z.object({
        key: z.string(),
        label: z.string(),
        help: z.string().optional(),
        modes: z.array(PlayModeSchema).optional(),
        kind: z.literal('int'),
        def: z.number().int(),
        min: z.number().int(),
        max: z.number().int(),
    }),
    z.object({
        key: z.string(),
        label: z.string(),
        help: z.string().optional(),
        modes: z.array(PlayModeSchema).optional(),
        kind: z.literal('text'),
        def: z.string(),
        maxLen: z.number().int().optional(),
    }),
])

export type PlayOptionT = z.infer<typeof PlayOptionSchema>

export const PlayOptionValueSchema = z.union([z.string(), z.number(), z.boolean()])

export const PlayOptionValuesSchema = z.record(z.string(), PlayOptionValueSchema)

export type PlayOptionValuesT = z.infer<typeof PlayOptionValuesSchema>

/**
 * What an app declares about being played, or null when it declares nothing.
 *
 * `directPlay` is the field the Apps tab exists to respect. It answers a
 * different question from `modes`: a game can be perfectly launchable and still
 * only make sense with a server chosen, in which case pressing Play on the GAME
 * has nowhere to go. The server enforces it in `/play/launch` as well — this
 * field only decides what is drawn, and a client that ignores it gets a `null`.
 */
export const AppPlaySchema = z.object({
    /** In preference order. Never empty — a null `play` means "no modes". */
    modes: z.array(PlayModeSchema),
    directPlay: z.boolean(),
    /** Operator-written note, shown above the launch options. */
    help: z.string().nullable(),
    options: z.array(PlayOptionSchema).default([]),
})

export type AppPlayT = z.infer<typeof AppPlaySchema>

/* ---------------------------------------------- Finding a server by address */

/**
 * Look one server up by the address somebody was handed.
 *
 * The other half of the `tmc://play/<host>:<port>` deep link. A link is an
 * address and nothing else — that is what makes it a link somebody can paste
 * into a message — and the app has to turn that into "which game is this, and
 * is it up?" before it can draw a join screen worth pressing.
 *
 * **The caller already knows the address**, which is what makes this endpoint
 * unremarkable: it discloses which listed server is at an address the asker
 * typed, and every one of those rows is on the public browser already. A server
 * that hides its network details is still FOUND here — hiding the address stops
 * us publishing it, and cannot un-tell somebody who was handed it — but the
 * answer never echoes an address back.
 */
export const ServerLookupQuerySchema = z.object({
    /** Hostname or IP literal, unbracketed. The app strips the brackets. */
    host: z.string().min(1).max(255),
    /**
     * Optional, because `tmc://play/example.com` is a legitimate link. Absent
     * matches any port on that host, and a host running two servers then
     * answers with the first — which the app renders as a choice rather than
     * guessing.
     */
    port: z.coerce.number().int().min(1).max(65535).optional(),
})

export type ServerLookupQueryT = z.input<typeof ServerLookupQuerySchema>

export const ServerLookupResponseSchema = z.object({
    /** Every listed server at that address. Empty when none is. */
    servers: z.array(ContentSummarySchema).max(10).default([]),
})

export type ServerLookupResponseT = z.infer<typeof ServerLookupResponseSchema>

/* ------------------------------------------------------ Installing a game */

/**
 * Which machine a native build is for. Mirrors `AppBuildPlatform`.
 *
 * ARCHITECTURE IS PART OF THE VALUE. `AppPlatform` — the descriptive "where is
 * this game available" enum the website draws on an app page — says
 * `PLATFORM_WINDOWS`, and that is a fine answer to that question. It is not an
 * answer to this one: the app matches on these values to pick a file it is
 * about to execute, and an x64 archive handed to an ARM machine either refuses
 * to start or runs emulated at a cost nobody chose.
 *
 * The app maps its own platform to exactly one of these and asks for that one.
 * A value it does not recognise is a platform it is not running on, so an older
 * app meeting a newer target simply finds no build rather than failing.
 */
export const AppBuildPlatformVals = [
    'WINDOWS_X64',
    'WINDOWS_ARM64',
    'LINUX_X64',
    'LINUX_ARM64',
    /** One universal binary, covering Intel and Apple Silicon. */
    'MACOS_UNIVERSAL',
    'ANDROID_ARM64',
    'IOS_ARM64',
] as const

export const AppBuildPlatformSchema = z.enum(AppBuildPlatformVals)
export type AppBuildPlatformT = (typeof AppBuildPlatformVals)[number]

/**
 * How the artifact is packed. Mirrors `AppBuildFormat`.
 *
 * `RAW` is not a degenerate archive: an `.apk` and a single static binary are
 * installed exactly as served, and unpacking either would destroy the thing
 * being installed.
 */
export const AppBuildFormatVals = ['ZIP', 'TAR_GZ', 'RAW'] as const
export const AppBuildFormatSchema = z.enum(AppBuildFormatVals)
export type AppBuildFormatT = (typeof AppBuildFormatVals)[number]

/**
 * One line in the catalogue saying a build EXISTS — never where it is.
 *
 * The address, the checksum and the launch arguments are answered by
 * `GET /apps/:id/build`, one platform at a time. That split is the app's own
 * rule about ids and URLs applied to the catalogue: a device that browses the
 * whole app list must not come away holding download addresses for every game
 * on every platform, and a card only needs to know whether to draw an Install
 * button and how large the download is.
 */
export const AppBuildRefSchema = z.object({
    platform: AppBuildPlatformSchema,
    version: z.string(),
    sizeBytes: z.number().int().nonnegative(),
})

export type AppBuildRefT = z.infer<typeof AppBuildRefSchema>

/**
 * What an app declares about being INSTALLED, or null when it publishes no
 * native build at all.
 *
 * Deliberately separate from {@link AppPlaySchema}. That one is about launching
 * something that already exists — a loader in a window, a deep link to a client
 * somebody has. This is about putting software on a disk, which is a different
 * question with a different answer for the same app: a game can be perfectly
 * playable in the browser and have nothing to install, and vice versa.
 */
export const AppInstallSchema = z.object({
    /** Every target with a published, enabled build. Never empty. */
    builds: z.array(AppBuildRefSchema),
})

export type AppInstallT = z.infer<typeof AppInstallSchema>

/**
 * One resolved build — the answer to `GET /apps/:id/build?platform=…`.
 *
 * The same shape as `/play/launch`: every decision is made server-side and the
 * device is told the answer, `null` meaning "not from here" for every refusal
 * together. Naming which refusal applied would tell a caller which column to
 * probe and would not change what the app does with the answer.
 */
export const NativeBuildSchema = z.object({
    appId: z.number().int(),
    platform: AppBuildPlatformSchema,
    version: z.string(),
    /** Absolute and `https`. The app re-checks the scheme before fetching it. */
    url: z.string(),
    /** Lower-case hex SHA-256. Never optional — the installer refuses without it. */
    sha256: z.string(),
    sizeBytes: z.number().int().nonnegative(),
    format: AppBuildFormatSchema,
    /** Relative path of the executable inside the archive; null for `RAW`. */
    entry: z.string().nullable(),
    /**
     * The launch argument template, one element per argument.
     *
     * An ARRAY rather than a command line, because the app has no shell to split
     * one with — a server name containing a space has to arrive as one argument
     * and not as two. Tokens are the `PLAY_URI_TOKENS` vocabulary: `{host}`,
     * `{port}`, `{app}`, `{appId}`, `{serverId}`, `{locale}`, `{opt:<key>}`.
     */
    args: z.array(z.string()).default([]),
    /** The oldest app version that may install this, or null for any. */
    minClientVersion: z.string().nullable(),
    /** What changed, shown beside the update button. Plain text. */
    notes: z.string().nullable(),
})

export type NativeBuildT = z.infer<typeof NativeBuildSchema>

export const AppBuildQuerySchema = z.object({
    /**
     * The machine asking. Required, and there is no "give me all of them":
     * the whole reason this is a second request is that a device receives one
     * download address, for itself.
     */
    platform: AppBuildPlatformSchema,
})

export type AppBuildQueryT = z.input<typeof AppBuildQuerySchema>

/**
 * Counts, per app, for the catalogue grid.
 *
 * Whole numbers with a zero default rather than nullable: "we did not count" and
 * "there are none" are the same fact to a card that has to draw a number, and a
 * nullable count means every caller writes the same `?? 0`.
 */
export const AppCountsSchema = z.object({
    mods: z.number().int().nonnegative().default(0),
    assets: z.number().int().nonnegative().default(0),
    servers: z.number().int().nonnegative().default(0),
    /** Players on those servers right now, summed. */
    players: z.number().int().nonnegative().default(0),
})

/** One row in the Apps tab. */
export const AppSummarySchema = z.object({
    id: z.number().int(),
    name: z.string(),
    /** URL slug, when it has a usable one. */
    slug: z.string().nullable(),
    description: z.string().nullable(),
    type: AppTypeSchema,
    images: ImageSetSchema,
    isOfficial: z.boolean(),
    /** Every integration the app claims. */
    integrations: z.array(AppIntegrationSchema).default([]),
    /** True for `GAME`, `GAME_ENGINE` and `VOIP` — see `AppHasServers`. */
    hasServers: z.boolean(),
    /** The engine this app is built on, when it names one. */
    engine: RefSchema.nullable().default(null),
    counts: AppCountsSchema,
    /** Null when the app is not playable at all. */
    play: AppPlaySchema.nullable().default(null),
    /** Null when the app publishes no native build for any platform. */
    install: AppInstallSchema.nullable().default(null),
    webUrl: z.string(),
})

export type AppSummaryT = z.infer<typeof AppSummarySchema>

/** How the Apps tab orders and narrows the catalogue. */
export const AppSortVals = ['players', 'servers', 'content', 'name'] as const
export const AppSortSchema = z.enum(AppSortVals)
export type AppSortT = (typeof AppSortVals)[number]

export const AppListQuerySchema = z.object({
    search: z.string().max(120).optional(),
    /**
     * Fetch specific apps by id, ignoring every other filter but the sort.
     *
     * The app's launcher needs the play vocabulary for ONE game — the one whose
     * server somebody just pressed Play on — and searching by name for it is
     * the kind of lookup that works until two games share a word. Bounded at
     * the page limit, so it cannot become a way to dump the catalogue in one
     * request that skips the cursor.
     */
    ids: z.array(z.coerce.number().int().positive()).max(100).optional(),
    /**
     * Fetch apps by URL slug, the same way `ids` fetches them by id.
     *
     * The app's plugin folders are named after the slug (`plugins/app/<slug>/`)
     * and its detection rules therefore produce slugs, while everything the app
     * stores — a game directory, a sandbox — is keyed by the numeric id. Nothing
     * else could translate: the device only learns the mapping by already
     * having a sandbox or a subscription for the game, which a machine that has
     * just been scanned for the first time does not.
     */
    slugs: z.array(z.string().max(120)).max(100).optional(),
    type: AppTypeSchema.optional(),
    /** Only apps that can be launched from here. */
    playable: QueryBool.optional(),
    /** Only apps the app can install mods for (`TMC_APP_MANAGE`). */
    managed: QueryBool.optional(),
    sort: AppSortSchema.default('players'),
    cursor: z.string().max(64).optional(),
    limit: z.coerce.number().int().min(1).max(100).default(50),
})

export type AppListQueryT = z.input<typeof AppListQuerySchema>

export const AppListResponseSchema = z.object({
    apps: z.array(AppSummarySchema),
    nextCursor: z.string().nullable(),
    total: z.number().int().nonnegative().nullable(),
    /**
     * Whether the play centre is switched on at all.
     *
     * Separate from "no app on this page is playable", exactly as the website's
     * `anyPlayable` is: an operator switching the feature off must not present
     * to a user as every game having lost its Play button.
     */
    playEnabled: z.boolean().default(false),
})

export type AppListResponseT = z.infer<typeof AppListResponseSchema>

// ----------------------------------------------------------- Launching one

/**
 * The descriptor a web loader is handed. Mirrors `GameBootDescriptorT`.
 *
 * Passed to the loader by assignment to a global rather than on the script's
 * query string — the app inherits that decision from the website's player, and
 * for the same reason: a server address on a URL ends up in every cache and
 * history entry that URL touches.
 */
export const GameBootServerSchema = z.object({
    id: z.number().int(),
    name: z.string().nullable(),
    host: z.string().nullable(),
    ip4: z.string().nullable(),
    ip6: z.string().nullable(),
    port: z.number().int().nullable(),
    portQuery: z.number().int().nullable(),
    hostName: z.string().nullable(),
    map: z.string().nullable(),
    gameMode: z.string().nullable(),
    connectUrl: z.string().nullable(),
})

/** Mirrors `AppGraphicsType`. Descriptive — nothing is gated on it. */
export const AppGraphicsVals = [
    'TWO_D',
    'TWO_HALF_D',
    'THREE_D',
    'VR',
    'TEXT',
    'MIXED',
    'OTHER',
] as const

export const AppGraphicsSchema = z.enum(AppGraphicsVals)

/** Which avatar representation a game calls for. Never null — see `AvatarKindT`. */
export const AvatarKindVals = ['model', 'flat', 'none'] as const
export const AvatarKindSchema = z.enum(AvatarKindVals)

/**
 * Where the published web build lives, or null when nothing is published yet.
 *
 * `origin` is published separately from `url` because getting the second one
 * wrong is a security bug rather than a cosmetic one: the frame is pointed at
 * `url`, and every `postMessage` carrying a credential must be targeted at
 * `origin` and never at `'*'`.
 */
export const GameBootGameSchema = z.object({
    url: z.string(),
    origin: z.string(),
    base: z.string(),
    build: z.string(),
})

/** Mirrors `ServerAuthModeVals` — the mode IN FORCE, so never `SERVER`. */
export const GameAuthModeVals = ['NONE', 'TMC', 'CUSTOM'] as const
export const GameAuthModeSchema = z.enum(GameAuthModeVals)

/**
 * Who is playing, or how the game should find out.
 *
 * Always present and never null: a game that uses no identity gets
 * `{ mode: 'NONE', guest: true }`, which is a complete answer rather than an
 * absence every reader has to interpret.
 *
 * `handoff.code` is a single-use, sixty-second credential redeemed at
 * `redeemUrl` (`POST /auth/handoff`). It is NOT a device token and must never
 * be stored: what it mints is bound to one app and refused by every route that
 * has not opted in.
 */
export const GameAuthSchema = z.object({
    mode: GameAuthModeSchema,
    guest: z.boolean(),
    /** Whether the SERVER chose this rather than the app. */
    fromServer: z.boolean(),
    issuerUrl: z.string().nullable(),
    signedIn: z.boolean(),
    handoff: z
        .object({
            code: z.string(),
            expiresIn: z.number().int(),
            redeemUrl: z.string(),
        })
        .nullable()
        .default(null),
})

export const GameBootSchema = z.object({
    version: z.literal(1),
    app: z.object({
        id: z.number().int(),
        name: z.string(),
        appId: z.string().nullable(),
        gameType: z.string().nullable(),
        /*
         * ADDED to version 1 rather than bumping it, on the descriptor's own
         * rule: the version is bumped when an existing field's MEANING changes,
         * and a reader that predates a new key simply does not read it. Both
         * carry a default for the same reason — an older server answering an
         * app built against this schema must not fail validation over a key it
         * has never heard of.
         */
        graphics: AppGraphicsSchema.nullable().default(null),
        avatarKind: AvatarKindSchema.default('flat'),
    }),
    game: GameBootGameSchema.nullable().default(null),
    server: GameBootServerSchema.nullable(),
    auth: GameAuthSchema.default({
        mode: 'NONE',
        guest: true,
        fromServer: false,
        issuerUrl: null,
        signedIn: false,
        handoff: null,
    }),
    party: z
        .object({
            id: z.string(),
            name: z.string().nullable(),
            maxUsers: z.number().int(),
        })
        .nullable(),
    locale: z.string(),
    options: PlayOptionValuesSchema.default({}),
})

export type GameBootT = z.infer<typeof GameBootSchema>

export const PlayLaunchRequestSchema = z.object({
    appId: z.number().int().positive(),
    mode: PlayModeSchema,
    serverId: z.number().int().positive().optional(),
    locale: z.string().max(16).default('en'),
    options: PlayOptionValuesSchema.optional(),
})

/**
 * A resolved launch, or `null` when there is not one.
 *
 * `null` is the whole refusal vocabulary and it covers every reason: the app is
 * hidden, the feature is off, the mode is not available, the server belongs to
 * another game, or `directPlay` is off and no server was named. Naming which
 * would tell a caller which columns to probe, and none of them changes what the
 * app does — it says the game cannot be started from here and points at its
 * page, which is where the servers are.
 */
export const PlayLaunchResponseSchema = z
    .discriminatedUnion('mode', [
        z.object({
            mode: z.literal('web'),
            /** The loader script. Absolute, and same-origin with the site. */
            loaderUrl: z.string(),
            boot: GameBootSchema,
        }),
        z.object({
            mode: z.literal('app'),
            /** The resolved deep link, tokens already substituted. */
            uri: z.string(),
        }),
    ])
    .nullable()

export type PlayLaunchResponseT = z.infer<typeof PlayLaunchResponseSchema>
