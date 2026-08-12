import { z } from 'zod'

/**
 * Mirrors of the Rust types that cross the IPC boundary.
 *
 * Same rule as the API contract: these are the SECOND copy of a shape whose
 * first copy is in `src-tauri/src`. Keeping them in one file makes the sync
 * obligation visible, and `call()` parses through them so a drift fails at the
 * boundary rather than deep in a component.
 */

export const SessionUserSchema = z.object({
    id: z.string(),
    name: z.string().nullable(),
    username: z.string().nullable(),
    avatar: z.string().nullable(),
    role: z.string().nullable(),
})

export type SessionUserT = z.infer<typeof SessionUserSchema>

export const SessionSnapshotSchema = z.object({
    signedIn: z.boolean(),
    pending: z.boolean(),
    user: SessionUserSchema.nullable(),
})

export type SessionSnapshotT = z.infer<typeof SessionSnapshotSchema>

export const PendingLoginSchema = z.object({
    userCode: z.string(),
    verificationUri: z.string(),
    verificationUriComplete: z.string(),
    expiresIn: z.number(),
    interval: z.number(),
})

export type PendingLoginT = z.infer<typeof PendingLoginSchema>

export const PollOutcomeSchema = z.object({
    status: z.enum(['pending', 'signedIn', 'denied', 'expired']),
    user: SessionUserSchema.nullable(),
})

/**
 * Which site this build talks to.
 *
 * Reported by Rust, never chosen here: the base is fixed at build time and, in
 * a debug build only, overridable through `TMC_API_BASE` in the environment.
 * There is no command that sets it, and adding one would hand any script in a
 * rendered mod description the ability to aim the next bearer token at a host
 * of its choosing.
 */
export const ApiEnvSchema = z.object({
    base: z.string(),
    isProd: z.boolean(),
    version: z.string(),
})

export type ApiEnvT = z.infer<typeof ApiEnvSchema>

// ----------------------------------------------------------------- Settings

export const AppSettingsSchema = z.object({
    version: z.number(),

    theme: z.string(),
    uiScale: z.number(),
    compactCards: z.boolean(),
    showNsfw: z.boolean(),

    verboseLogging: z.boolean(),
    crashReports: z.boolean(),

    downloadDir: z.string().nullable(),
    gameDirs: z.record(z.string(), z.string()),
    autoUpdateCheck: z.boolean(),
    liveLatency: z.boolean(),
    latencyIntervalMs: z.number(),
    latencyConcurrency: z.number(),
    minimiseToTray: z.boolean(),

    requireSignedPlugins: z.boolean(),
    confirmEveryRun: z.boolean(),
})

export type AppSettingsT = z.infer<typeof AppSettingsSchema>

// ------------------------------------------------------------------ Logging

export const LogLevelSchema = z.enum(['debug', 'info', 'security', 'warn', 'error'])
export type LogLevelT = z.infer<typeof LogLevelSchema>

export const LogScopeSchema = z.enum([
    'auth',
    'api',
    'plugin',
    'install',
    'network',
    'settings',
    'app',
])

export const LogEntrySchema = z.object({
    at: z.string(),
    level: LogLevelSchema,
    scope: LogScopeSchema,
    event: z.string(),
    message: z.string(),
    plugin: z.string().optional(),
    data: z.record(z.string(), z.unknown()).optional(),
})

export type LogEntryT = z.infer<typeof LogEntrySchema>

// ------------------------------------------------------------------ Network

export const PingResultSchema = z.object({
    host: z.string(),
    port: z.number(),
    rttMs: z.number().nullable(),
    received: z.number(),
    attempts: z.number(),
})

export type PingResultT = z.infer<typeof PingResultSchema>

// ------------------------------------------------------------ Live queries

/**
 * Mirrors `tmc_core::net::query::QueryProtocol`.
 *
 * A superset of the contract's `QueryProtocolVals`: Rust also carries
 * `TCP_ONLY`, the fallback it picks itself when a game declares nothing this
 * build implements. The server never sends that value; Rust returns it.
 */
export const RustQueryProtocolSchema = z.enum([
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
    'TCP_ONLY',
])

export type RustQueryProtocolT = z.infer<typeof RustQueryProtocolSchema>

export const PlayerEntrySchema = z.object({
    name: z.string(),
    score: z.number().nullable(),
    duration: z.number().nullable(),
    ping: z.number().nullable(),
})

export type PlayerEntryT = z.infer<typeof PlayerEntrySchema>

export const ServerQueryResultSchema = z.object({
    online: z.boolean(),
    rttMs: z.number(),
    protocol: RustQueryProtocolSchema,

    name: z.string().nullable(),
    map: z.string().nullable(),
    game: z.string().nullable(),
    version: z.string().nullable(),

    players: z.number().nullable(),
    maxPlayers: z.number().nullable(),
    bots: z.number().nullable(),

    password: z.boolean().nullable(),
    secure: z.boolean().nullable(),

    playerList: z.array(PlayerEntrySchema).default([]),
    rules: z.record(z.string(), z.string()).default({}),

    queriedPort: z.number(),
})

export type ServerQueryResultT = z.infer<typeof ServerQueryResultSchema>

export const QueryOutcomeSchema = z.object({
    id: z.string(),
    key: z.string(),
    result: ServerQueryResultSchema.optional(),
    error: z.string().optional(),
})

export type QueryOutcomeT = z.infer<typeof QueryOutcomeSchema>

export const LatencySampleSchema = z.object({
    at: z.number(),
    rttMs: z.number().nullable(),
})

export type LatencySampleT = z.infer<typeof LatencySampleSchema>

export const LatencySeriesSchema = z.object({
    key: z.string(),
    samples: z.array(LatencySampleSchema),
    last: z.number().nullable(),
    min: z.number().nullable(),
    max: z.number().nullable(),
    avg: z.number().nullable(),
    jitter: z.number().nullable(),
    reliability: z.number(),
    /** Middle successful sample — the honest centre of a bimodal series. */
    median: z.number().nullable(),
    /**
     * Evidence that something is answering this query from a cache.
     *
     * A cache in front of a query port is fast on every HIT and pays the full
     * round trip on every miss, so the series goes bimodal. `ratio` is how much
     * slower the miss cluster is, `fastShare` how often the cache answered.
     * Null until there are enough samples to say anything.
     *
     * NOT an accusation — a CDN, a proxy and a game that answers from memory
     * all look like this. It is shown so a player reads "12ms" with the right
     * amount of trust.
     */
    cache: z
        .object({
            ratio: z.number(),
            fastShare: z.number(),
            fastMs: z.number(),
            slowMs: z.number(),
        })
        .nullable(),
})

export type LatencySeriesT = z.infer<typeof LatencySeriesSchema>

// ------------------------------------------------------------------ Plugins

export const PluginKindSchema = z.enum(['installer', 'serverQuery', 'theme'])
export type PluginKindT = z.infer<typeof PluginKindSchema>

export const PluginRecordSchema = z.object({
    id: z.string(),
    name: z.string(),
    version: z.string(),
    author: z.string(),
    description: z.string().nullable(),
    kinds: z.array(PluginKindSchema),
    apps: z.array(z.number()),
    permissions: z.array(z.string()),
    approvedFingerprint: z.string(),
    approvedAt: z.string(),
    enabled: z.boolean(),
    dir: z.string(),
    needsReapproval: z.boolean(),
})

export type PluginRecordT = z.infer<typeof PluginRecordSchema>

export const PluginPreviewSchema = z.object({
    id: z.string(),
    name: z.string(),
    version: z.string(),
    author: z.string(),
    description: z.string().nullable(),
    kinds: z.array(PluginKindSchema),
    permissions: z.array(z.string()),
    fingerprint: z.string(),
    dir: z.string(),
})

export type PluginPreviewT = z.infer<typeof PluginPreviewSchema>

export const RunReportSchema = z.object({
    ok: z.boolean(),
    stepsTotal: z.number(),
    stepsRun: z.number(),
    applied: z.array(z.string()),
    failedAt: z.number().optional(),
    error: z.string().optional(),
})

export type RunReportT = z.infer<typeof RunReportSchema>

export const ThemeSchema = z.object({
    label: z.string(),
    base: z.string().nullable().optional(),
    tokens: z.record(z.string(), z.string()),
})

export type ThemeT = z.infer<typeof ThemeSchema>

// ------------------------------------------------------------------ Library

export const LibraryStateSchema = z.enum([
    'idle',
    'queued',
    'installing',
    'installed',
    'failed',
    'removing',
])

export type LibraryStateT = z.infer<typeof LibraryStateSchema>

/**
 * One row of the device's library.
 *
 * The server's fields and this device's, in one object — every screen that
 * shows one wants the other ("Cool Mod · v1.5 available · v1.4 installed" is a
 * single row), and splitting them would mean a join in the UI.
 *
 * Mirrors `LibraryRow` in `commands/library.rs`, which flattens `LibraryEntry`
 * and adds the two derived answers Rust is better placed to compute.
 */
export const LibraryRowSchema = z.object({
    id: z.string(),
    kind: z.enum(['asset', 'mod', 'collection']),
    itemId: z.number(),
    name: z.string(),
    description: z.string().nullable(),
    image: z.string().nullable(),
    webUrl: z.string(),
    appId: z.number().nullable(),
    appName: z.string().nullable(),
    appSlug: z.string().nullable(),

    autoUpdate: z.boolean(),
    notifyUpdates: z.boolean(),
    paused: z.boolean(),
    viaCollectionId: z.number().nullable(),
    installable: z.boolean(),

    latestReleaseId: z.number().nullable(),
    latestVersion: z.string().nullable(),
    fileUrl: z.string().nullable(),
    fileName: z.string().nullable(),
    fileSize: z.number().nullable(),
    fileSha256: z.string().nullable(),
    updatedAt: z.string(),

    installedReleaseId: z.number().nullable(),
    installedVersion: z.string().nullable(),
    installedInstallId: z.number().nullable(),
    installedAt: z.string().nullable(),
    installedFiles: z.array(z.string()),
    state: LibraryStateSchema,
    lastError: z.string().nullable(),

    /** Derived: is a newer release available than the one on disk? */
    updateAvailable: z.boolean(),
    /** Derived: is there a rule on THIS machine that could install it? */
    hasRule: z.boolean(),
    /**
     * A grouping rather than something with files.
     *
     * Subscribing to a collection creates a real subscription per member, and
     * those are what install; the collection row names the group. Without this
     * the UI would report "no install rule" for every collection — an error
     * about something that was never going to happen.
     */
    isContainer: z.boolean(),
})

export type LibraryRowT = z.infer<typeof LibraryRowSchema>

export const SyncReportSchema = z.object({
    received: z.number(),
    added: z.number(),
    updated: z.number(),
    removed: z.number(),
    full: z.boolean(),
    toInstall: z.array(z.string()),
    toUninstall: z.array(z.string()),
    toForget: z.array(z.string()),
})

export type SyncReportT = z.infer<typeof SyncReportSchema>

export const InstallOutcomeSchema = z.object({
    id: z.string(),
    ok: z.boolean(),
    rule: z.string().nullable(),
    report: RunReportSchema.nullable(),
    error: z.string().nullable(),
})

export type InstallOutcomeT = z.infer<typeof InstallOutcomeSchema>

/** One app-scoped rule, for Settings → Plugins. */
export const RuleInfoSchema = z.object({
    source: z.string(),
    kind: z.enum(['manageMod', 'manageAsset', 'manageCollection', 'launch']),
    label: z.string().nullable(),
    description: z.string().nullable(),
    extensions: z.array(z.string()),
    loaders: z.array(z.string()),
    manages: z.boolean(),
})

export type RuleInfoT = z.infer<typeof RuleInfoSchema>

/**
 * What launching an install would run, resolved but NOT run.
 *
 * The whole reason a declarative launcher is acceptable: the user sees the
 * exact program, arguments and working directory before anything starts.
 * `command` is a readable one-liner for DISPLAY and must never be fed to a
 * shell — the real launch uses the argv vector.
 */
export const LaunchPlanSchema = z.object({
    rule: z.string(),
    program: z.string().nullable(),
    uri: z.string().nullable(),
    args: z.array(z.string()),
    cwd: z.string().nullable(),
    env: z.record(z.string(), z.string()),
    hideWindow: z.boolean(),
})

export const LaunchPreviewSchema = z.object({
    plan: LaunchPlanSchema,
    command: z.string(),
})

export type LaunchPreviewT = z.infer<typeof LaunchPreviewSchema>

// ------------------------------------------------------------- Folder picker

export const DirEntrySchema = z.object({
    name: z.string(),
    path: z.string(),
})

export type DirEntryT = z.infer<typeof DirEntrySchema>

export const DirListingSchema = z.object({
    path: z.string(),
    name: z.string(),
    parent: z.string().nullable(),
    entries: z.array(DirEntrySchema),
    /** The listing was cut short by the Rust-side cap. */
    truncated: z.boolean(),
    /** The OS refused the listing. Not an error: the picker still shows the
     *  crumb so the user can go back up. */
    denied: z.string().optional(),
})

export type DirListingT = z.infer<typeof DirListingSchema>

export const DirRootSchema = z.object({
    label: z.string(),
    path: z.string(),
})

export type DirRootT = z.infer<typeof DirRootSchema>
