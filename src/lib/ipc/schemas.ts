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

    /** Ceiling across every download, bytes per second. `0` is unlimited. */
    downloadLimitBps: z.number(),
    downloadConcurrency: z.number(),
    downloadKeepHistory: z.boolean(),

    requireSignedPlugins: z.boolean(),
    confirmEveryRun: z.boolean(),
    /**
     * Whether the app may open a game's own settings files.
     *
     * ON by default — editing a loader's `.cfg` is ordinary work for a mod
     * manager. It exists because it is the one feature that lets the webview
     * name a file to be written, and it removes exactly those commands and
     * nothing else.
     */
    allowConfigEditing: z.boolean(),
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

/**
 * Who vouched for a bundle.
 *
 * Three states, not two. "Signed by somebody you have not trusted" is not the
 * same fact as "not signed": the first is either a publisher whose key the user
 * has not added yet or a tampered bundle, and both deserve saying out loud
 * rather than folding into silence.
 *
 * Rust serialises this as `{ state, keyId }`, with `keyId` present only for
 * `trusted`.
 */
export const SignatureStateSchema = z.union([
    z.object({ state: z.literal('unsigned') }),
    z.object({ state: z.literal('untrusted') }),
    z.object({ state: z.literal('trusted'), keyId: z.string() }),
])

export type SignatureStateT = z.infer<typeof SignatureStateSchema>

export const TrustedKeySchema = z.object({
    id: z.string(),
    label: z.string(),
    /** 32 bytes, hex. */
    publicKey: z.string(),
    addedAt: z.string(),
})

export type TrustedKeyT = z.infer<typeof TrustedKeySchema>

/**
 * What an update check found.
 *
 * A NOTICE, not an updater — there is no artifact, signature or checksum here
 * because the app does not update itself. `outdated` is computed in Rust by
 * comparing dotted numeric components, because a string compare puts `1.10.0`
 * before `1.9.0` and the symptom is a banner that never appears or never goes
 * away.
 */
export const UpdateCheckSchema = z.object({
    current: z.string(),
    latest: z.string().nullable(),
    /** Absolute https, validated server-side and again in Rust. */
    download: z.string().nullable(),
    outdated: z.boolean(),
})

export type UpdateCheckT = z.infer<typeof UpdateCheckSchema>

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
    /**
     * Defaulted, because a registry written before signatures existed has no
     * such field — and those plugins genuinely are unsigned.
     */
    signature: SignatureStateSchema.default({ state: 'unsigned' }),
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
    signature: SignatureStateSchema,
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
    /**
     * Set when the sandbox being launched deployed virtually, so the game has
     * to be started by the app itself with its filesystem carried in. Display
     * only here — the injection happens in Rust, and the webview cannot ask
     * for it or point it anywhere.
     */
    vfs: z.object({ blob: z.string(), root: z.string() }).nullable().default(null),
})

// ----------------------------------------------------------------- Sessions

/**
 * How a game was started, which decides what can be known about it.
 *
 * `handoff` is the honest answer for a `steam://` launch: the OS opener started
 * a launcher which started the game, so there is no process of ours, no exit
 * code and NO playtime. The UI must not draw a duration for one — see
 * `tmc_core::session`.
 */
export const SessionKindSchema = z.enum(['process', 'handoff', 'web'])
export type SessionKindT = z.infer<typeof SessionKindSchema>

export const SessionSchema = z.object({
    id: z.number(),
    kind: SessionKindSchema,
    appId: z.number().nullable(),
    appSlug: z.string().nullable(),
    label: z.string(),
    sandboxId: z.number().nullable(),
    installId: z.number().nullable(),
    startedMs: z.number(),
    /** Absent while it is still running. */
    endedMs: z.number().nullable(),
    pid: z.number().nullable(),
    exitCode: z.number().nullable(),
    stoppedByUser: z.boolean(),
    logPath: z.string().nullable(),
})

export type SessionT = z.infer<typeof SessionSchema>

/** A finished launch, from the durable history. */
export const SessionRowSchema = z.object({
    id: z.number(),
    kind: z.string(),
    appId: z.number().nullable(),
    appSlug: z.string().nullable(),
    label: z.string(),
    sandboxId: z.number().nullable(),
    installId: z.number().nullable(),
    startedMs: z.number(),
    endedMs: z.number().nullable(),
    /** Measured seconds. Always 0 for a launch whose duration is unknowable. */
    seconds: z.number(),
    exitCode: z.number().nullable(),
    stoppedByUser: z.boolean(),
})

export type SessionRowT = z.infer<typeof SessionRowSchema>

export const PlayTotalsSchema = z.object({
    seconds: z.number(),
    launches: z.number(),
    lastPlayedMs: z.number().nullable(),
})

export type PlayTotalsT = z.infer<typeof PlayTotalsSchema>

export const PlaytimeSummarySchema = z.object({
    /** Keyed by TMC app id as a string — JSON object keys always are. */
    byApp: z.record(z.string(), PlayTotalsSchema),
    bySandbox: z.record(z.string(), PlayTotalsSchema),
})

export type PlaytimeSummaryT = z.infer<typeof PlaytimeSummarySchema>

export const FlushReportSchema = z.object({
    reported: z.number(),
    seconds: z.number(),
    deferred: z.number(),
})

export const LaunchPreviewSchema = z.object({
    plan: LaunchPlanSchema,
    command: z.string(),
    /**
     * The session that was opened, on the paths that actually start something.
     *
     * Absent from a PREVIEW, which is the point of a preview. Present on a
     * launch so the caller can watch that session rather than guess which of
     * the running ones it just created — two launches of the same game a
     * second apart are indistinguishable by app id alone.
     */
    session: SessionSchema.optional(),
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

// ----------------------------------------------------------------- Sandboxes

/** Mirrors `tmc_core::library::sandbox::Environment`. */
export const SandboxEnvironmentSchema = z.enum(['client', 'server', 'shared'])
export type SandboxEnvironmentT = z.infer<typeof SandboxEnvironmentSchema>

/** Mirrors `tmc_core::deploy::Strategy`. */
export const DeployStrategySchema = z.enum([
    'direct',
    'hardlink',
    'symlink',
    'usvfs',
])
export type DeployStrategyT = z.infer<typeof DeployStrategySchema>

export const SandboxModSchema = z.object({
    modKey: z.string(),
    kind: z.string(),
    itemId: z.number(),
    name: z.string(),
    enabled: z.boolean(),
    priority: z.number(),
    /** Which release is downloaded into this sandbox's staging folder. */
    releaseId: z.number().nullable(),
    version: z.string().nullable(),
    stagedAt: z.string().nullable(),
    lastError: z.string().nullable(),
})

export type SandboxModT = z.infer<typeof SandboxModSchema>

export const SandboxRowSchema = z.object({
    id: z.number(),
    /** `AppInstall.id`, or null for a sandbox kept off the cloud. */
    remoteId: z.number().nullable(),
    appId: z.number(),
    appSlug: z.string().nullable(),
    appName: z.string().nullable(),
    name: z.string(),
    description: z.string().nullable(),
    environment: SandboxEnvironmentSchema,
    strategy: DeployStrategySchema,
    gameVersion: z.string().nullable(),
    loader: z.string().nullable(),
    preset: z.string().nullable(),
    isDefault: z.boolean(),
    cloudSync: z.boolean(),
    /** Keep this sandbox's mods at the newest release its subscriptions offer. */
    autoUpdate: z.boolean(),
    gameDir: z.string().nullable(),
    options: z.record(z.string(), z.unknown()),
    launchArgs: z.array(z.string()),
    launchEnv: z.record(z.string(), z.string()),
    deployedAt: z.string().nullable(),
    lastDeploy: z.unknown().nullable(),
    createdAt: z.string(),
    updatedAt: z.string(),
    mods: z.array(SandboxModSchema),

    // Derived by the command layer, so every screen agrees on the answer.
    needsDeploy: z.boolean(),
    deployedFiles: z.number(),
    /** Null is the UI's cue to ask for a game folder before anything else. */
    targetDir: z.string().nullable(),
})

export type SandboxRowT = z.infer<typeof SandboxRowSchema>

export const ConflictSchema = z.object({
    path: z.string(),
    winner: z.string(),
    losers: z.array(z.string()),
})

export const DeployReportSchema = z.object({
    requested: z.string(),
    used: z.string(),
    /** Set when the requested strategy was not possible here, and why. */
    fellBack: z.string().nullable(),
    dryRun: z.boolean(),
    placed: z.number(),
    reused: z.number(),
    removed: z.number(),
    restored: z.number(),
    backedUp: z.number(),
    conflicts: z.array(ConflictSchema),
    empty: z.array(z.string()),
    skipped: z.array(z.string()),
    warnings: z.array(z.string()),
    errors: z.array(z.string()),
})

export type DeployReportT = z.infer<typeof DeployReportSchema>

export const PurgeReportSchema = z.object({
    removed: z.number(),
    restored: z.number(),
    /** Files left alone because they are no longer the ones we deployed. */
    kept: z.array(z.string()),
    missing: z.number(),
    errors: z.array(z.string()),
})

export type PurgeReportT = z.infer<typeof PurgeReportSchema>

export const VerifyReportSchema = z.object({
    total: z.number(),
    intact: z.number(),
    missing: z.array(z.string()),
    changed: z.array(z.string()),
})

export type VerifyReportT = z.infer<typeof VerifyReportSchema>

export const StrategyReportSchema = z.object({
    strategy: z.string(),
    available: z.boolean(),
    /** Why not, when `available` is false — and a warning when it is true. */
    reason: z.string().nullable(),
})

export type StrategyReportT = z.infer<typeof StrategyReportSchema>

export const StageOutcomeSchema = z.object({
    modKey: z.string(),
    ok: z.boolean(),
    rule: z.string().nullable(),
    files: z.number(),
    error: z.string().nullable(),
})

export type StageOutcomeT = z.infer<typeof StageOutcomeSchema>

/** What a one-click install did, step by step. */
export const QuickInstallReportSchema = z.object({
    /** The account was subscribed to the item as part of this. */
    subscribed: z.boolean(),
    /** It was already in the sandbox and nothing was added. */
    alreadyPresent: z.boolean(),
    staged: StageOutcomeSchema.nullable(),
    deployed: DeployReportSchema.nullable(),
    /** Steps that did not run, and why. Never fatal on their own. */
    warnings: z.array(z.string()),
})

export type QuickInstallReportT = z.infer<typeof QuickInstallReportSchema>

// ----------------------------------------------------------------- Sharing

/**
 * A sandbox as it travels between machines.
 *
 * Note what is NOT in it: no game directory, no launch environment, no
 * deployment ledger, no staging paths. All four describe one computer, and a
 * code is by definition going to a different one — see `library::share`.
 */
export const SharedSandboxSchema = z.object({
    v: z.number(),
    appId: z.number(),
    appSlug: z.string().nullable(),
    appName: z.string().nullable(),
    name: z.string(),
    description: z.string().nullable(),
    environment: SandboxEnvironmentSchema,
    strategy: DeployStrategySchema,
    gameVersion: z.string().nullable(),
    loader: z.string().nullable(),
    preset: z.string().nullable(),
    options: z.record(z.string(), z.unknown()),
    launchArgs: z.array(z.string()),
    mods: z.array(
        z.object({
            kind: z.string(),
            itemId: z.number(),
            /** The name AS EXPORTED. Display only — somebody else chose it. */
            name: z.string(),
            enabled: z.boolean(),
            priority: z.number(),
        })
    ),
})

export type SharedSandboxT = z.infer<typeof SharedSandboxSchema>

// ----------------------------------------------------- Game settings files

/** Which root a config file lives under. Half of its address. */
export const FsRootSchema = z.enum(['gameDir', 'pluginData', 'downloads'])
export type FsRootT = z.infer<typeof FsRootSchema>

export const ConfigFileSchema = z.object({
    /** The declaring location's label, so the UI groups without a lookup. */
    group: z.string(),
    root: FsRootSchema,
    /**
     * Path relative to the ROOT, not to the location — that is what the Rust
     * side resolves, so sending anything else would need the location's own
     * path added back somewhere.
     */
    path: z.string(),
    name: z.string(),
    size: z.number(),
    modifiedMs: z.number().nullable(),
    /**
     * False for a file that is listed but cannot be opened here — too large,
     * or not text. Listed anyway, because "it is not here" and "it is here and
     * cannot be edited" are different answers.
     */
    editable: z.boolean(),
    reason: z.string().nullable(),
})

export type ConfigFileT = z.infer<typeof ConfigFileSchema>

export const SaveReportSchema = z.object({
    bytes: z.number(),
    /** Where the previous contents went. Null when the file was new. */
    backup: z.string().nullable(),
})

export type SaveReportT = z.infer<typeof SaveReportSchema>

export const ImportReportSchema = z.object({
    sandbox: SandboxRowSchema.nullable(),
    added: z.number(),
    /** Items that could not be added, one readable line each. */
    skipped: z.array(z.string()),
})

export type ImportReportT = z.infer<typeof ImportReportSchema>

/** One setting a game's launch rule understands — `apps::OptionSpec`. */
export const OptionSpecSchema = z.object({
    key: z.string(),
    label: z.string(),
    description: z.string().nullable().optional(),
    type: z.enum(['int', 'text', 'bool', 'select']),
    default: z.unknown().nullable().optional(),
    min: z.number().nullable().optional(),
    max: z.number().nullable().optional(),
    step: z.number().nullable().optional(),
    unit: z.string().nullable().optional(),
    choices: z.array(z.object({ value: z.string(), label: z.string() })),
    /** Empty means every environment. */
    environments: z.array(z.string()),
})

export type OptionSpecT = z.infer<typeof OptionSpecSchema>

export const PresetSpecSchema = z.object({
    id: z.string(),
    label: z.string(),
    description: z.string().nullable().optional(),
    environment: z.string().nullable().optional(),
    gameVersion: z.string().nullable().optional(),
    loader: z.string().nullable().optional(),
    strategy: z.string().nullable().optional(),
    options: z.record(z.string(), z.unknown()),
    launchArgs: z.array(z.string()),
})

export type PresetSpecT = z.infer<typeof PresetSpecSchema>

export const SandboxSpecSchema = z.object({
    deploy: z.object({
        defaultStrategy: z.string().nullable().optional(),
        supportedStrategies: z.array(z.string()),
        /** `kernel` is the one that matters — see the deploy engine. */
        antiCheat: z.string().nullable().optional(),
        modTargets: z.array(z.object({ type: z.string(), relPath: z.string() })),
        notes: z.array(z.string()),
    }),
    presets: z.array(PresetSpecSchema),
    options: z.array(OptionSpecSchema),
    detect: z
        .object({
            steamAppIds: z.array(z.string()),
            epicAppNames: z.array(z.string()),
            gogProductIds: z.array(z.string()),
            names: z.array(z.string()),
            markers: z.array(z.string()),
            paths: z.record(z.string(), z.array(z.string())),
        })
        .optional(),
})

export type SandboxSpecT = z.infer<typeof SandboxSpecSchema>

// ----------------------------------------------------------------- Downloads

export const DownloadStatusSchema = z.enum([
    'queued',
    'running',
    'paused',
    'done',
    'failed',
    'cancelled',
])

export type DownloadStatusT = z.infer<typeof DownloadStatusSchema>

export const DownloadSchema = z.object({
    id: z.string(),
    label: z.string(),
    url: z.string(),
    dest: z.string(),
    status: DownloadStatusSchema,
    /** Null until the server says, and it may never say. */
    total: z.number().nullable(),
    done: z.number(),
    speedBps: z.number(),
    etaSecs: z.number().nullable(),
    priority: z.number(),
    limitBps: z.number().nullable(),
    attempts: z.number(),
    error: z.string().nullable(),
    sha256: z.string().nullable(),
    /** Recent speeds, oldest first — what the graph draws. */
    samples: z.array(z.number()),
    queuedAt: z.string(),
    updatedAt: z.string(),
    meta: z.record(z.string(), z.string()),
})

export type DownloadT = z.infer<typeof DownloadSchema>

export const QueueSnapshotSchema = z.object({
    downloads: z.array(DownloadSchema),
    active: z.number(),
    speedBps: z.number(),
    limitBps: z.number().nullable(),
})

export type QueueSnapshotT = z.infer<typeof QueueSnapshotSchema>

// ---------------------------------------------------------------- Detection

export const DetectedGameSchema = z.object({
    /** `steam`, `epic`, `gog`, `xbox`, `ubisoft`, `ea`, `battlenet`, `folder`. */
    source: z.string(),
    name: z.string(),
    path: z.string(),
    launcherId: z.string().nullable(),
    launchUri: z.string().nullable(),
    sizeBytes: z.number().nullable(),
    /** The TMC game this was matched to, when a hint matched. */
    slug: z.string().nullable(),
    alreadySet: z.boolean(),
    replaces: z.string().nullable(),
})

export type DetectedGameT = z.infer<typeof DetectedGameSchema>

/** How far a hand-driven filesystem scan may go. Rust clamps every field. */
export const WalkLimitsSchema = z.object({
    /** Directory levels below each ticked root. */
    depth: z.number().int().min(0).max(8),
    /** Directories visited across the whole scan. */
    maxDirs: z.number().int().min(1).max(200_000),
    budgetSecs: z.number().int().min(1).max(300),
    /**
     * Descend into dot-directories.
     *
     * Worth offering rather than hard-coding: `~/.minecraft`,
     * `~/.local/share/Steam` and `~/.steam` are all real install locations, so
     * a scan of a Linux home with this off finds nothing at all.
     */
    hidden: z.boolean(),
})

export type WalkLimitsT = z.infer<typeof WalkLimitsSchema>

/**
 * Why a scan stopped.
 *
 * A scan that hit a limit and one that finished are different answers — the
 * first means "there may be more, look somewhere narrower" — and showing them
 * identically is how somebody concludes their game is undetectable when the
 * walk simply never reached it.
 */
export const WalkStopSchema = z.enum([
    'completed',
    'dirLimit',
    'timeLimit',
    'cancelled',
])

export type WalkStopT = z.infer<typeof WalkStopSchema>

export const ScanReportSchema = z.object({
    games: z.array(DetectedGameSchema),
    dirsVisited: z.number(),
    elapsedMs: z.number(),
    stop: WalkStopSchema,
    /** `[path, reason]` for each ticked root that could not be read. */
    unreadable: z.array(z.tuple([z.string(), z.string()])),
})

export type ScanReportT = z.infer<typeof ScanReportSchema>

/** One directory the walk is currently in. Emitted on `tmc://scan-progress`. */
export const ScanProgressSchema = z.object({
    dirs: z.number(),
    path: z.string(),
})

export type ScanProgressT = z.infer<typeof ScanProgressSchema>

export const ApplyOutcomeSchema = z.object({
    slug: z.string(),
    path: z.string(),
    ok: z.boolean(),
    error: z.string().optional(),
})

export type ApplyOutcomeT = z.infer<typeof ApplyOutcomeSchema>

// --------------------------------------------------------------------- RCON

export const RconProtocolSchema = z.enum(['source', 'frostbite'])
export type RconProtocolT = z.infer<typeof RconProtocolSchema>

/**
 * A saved server.
 *
 * Note what is absent: the password. The Rust type has no field for one, so no
 * refactor can start returning it — see `rcon::store`.
 */
export const RconServerSchema = z.object({
    id: z.number(),
    name: z.string(),
    host: z.string(),
    port: z.number(),
    protocol: RconProtocolSchema,
    serverId: z.number().nullable(),
    appId: z.number().nullable(),
    /** Whether one is stored, so the UI can prompt without being told what. */
    hasPassword: z.boolean(),
    lastUsedAt: z.string().nullable(),
    createdAt: z.string(),
})

export type RconServerT = z.infer<typeof RconServerSchema>

export const RconReplySchema = z.object({
    command: z.string(),
    output: z.string(),
    tookMs: z.number(),
})

export type RconReplyT = z.infer<typeof RconReplySchema>

export const RconHistorySchema = z.object({
    command: z.string(),
    output: z.string(),
    ok: z.boolean(),
    at: z.string(),
})

export type RconHistoryT = z.infer<typeof RconHistorySchema>

// ------------------------------------------------------------- Dependencies

/** Mirrors `library::dependency::Relation`, which mirrors Prisma's enum. */
export const DependencyRelationSchema = z.enum([
    'Required',
    'Optional',
    'Recommended',
    'Conflict',
])

export type DependencyRelationT = z.infer<typeof DependencyRelationSchema>

export const DependencyEdgeSchema = z.object({
    /** The item that HAS the dependency. */
    kind: z.string(),
    itemId: z.number(),
    /** The other end. */
    relKind: z.string(),
    relId: z.number(),
    relation: DependencyRelationSchema,
    name: z.string(),
    icon: z.string().nullable(),
    note: z.string().nullable(),
})

export type DependencyEdgeT = z.infer<typeof DependencyEdgeSchema>

export const DependencyConflictSchema = z.object({
    aKey: z.string(),
    aName: z.string(),
    bKey: z.string(),
    bName: z.string(),
    note: z.string().nullable(),
})

export const DependencyReportSchema = z.object({
    /** Required by something here, and not here. */
    missing: z.array(DependencyEdgeSchema),
    /** Recommended by something here, and not here. */
    suggested: z.array(DependencyEdgeSchema),
    conflicts: z.array(DependencyConflictSchema),
    /**
     * Items whose edges have never been fetched.
     *
     * Named rather than hidden: "no problems found" and "we did not look" are
     * different answers, and a screen showing the first when it means the
     * second is lying.
     */
    unchecked: z.array(z.string()),
})

export type DependencyReportT = z.infer<typeof DependencyReportSchema>

// ------------------------------------------------------------- Auto-updates

export const OutdatedSchema = z.object({
    sandboxId: z.number(),
    sandboxName: z.string(),
    modKey: z.string(),
    name: z.string(),
    /** Null when the staged release predates version strings being recorded. */
    fromVersion: z.string().nullable(),
    toVersion: z.string().nullable(),
    releaseId: z.number().nullable(),
})

export type OutdatedT = z.infer<typeof OutdatedSchema>

export const AutoUpdateReportSchema = z.object({
    checked: z.number(),
    updated: z.array(z.string()),
    redeployed: z.array(z.string()),
    /** `[item, reason]` for anything that did not make it. */
    failed: z.array(z.tuple([z.string(), z.string()])),
    /** Had an update, left alone, and why. */
    skipped: z.array(z.string()),
})

export type AutoUpdateReportT = z.infer<typeof AutoUpdateReportSchema>

// ------------------------------------------------- Imported (local) content

/**
 * How a mod that has no account behind it got onto this device.
 *
 * Kept because it is the first thing anybody asks about a row they do not
 * recognise, and because the four have different repair stories — a dropped
 * archive can be re-dropped, an adopted folder is still in the game directory,
 * one from another manager is still in that manager.
 */
export const LocalOriginSchema = z.enum(['dropped', 'folder', 'adopted', 'manager'])

export type LocalOriginT = z.infer<typeof LocalOriginSchema>

/**
 * What an imported archive's own `tmc.json` claimed, when it claimed an item on
 * THIS site.
 *
 * A link and nothing more: nothing about an imported mod is synced, updated or
 * reported because of it. Its use is the offer — "subscribe to this instead" —
 * which is what turns a file into something the app can keep current.
 */
export const LocalSourceSchema = z.object({
    kind: z.string(),
    itemId: z.number(),
    releaseId: z.number().nullish(),
    webUrl: z.string().nullish(),
})

export const LocalModSchema = z.object({
    id: z.number(),
    name: z.string(),
    appId: z.number().nullable(),
    appSlug: z.string().nullable(),
    version: z.string().nullable(),
    author: z.string().nullable(),
    notes: z.string().nullable(),
    origin: LocalOriginSchema,
    originLabel: z.string().nullable(),
    /**
     * Where its files land under the game folder. Editable, and editing it
     * MOVES the files — see `localPatch`.
     */
    relPath: z.string(),
    source: LocalSourceSchema.nullish(),
    files: z.number(),
    bytes: z.number(),
    addedAt: z.string(),
    updatedAt: z.string(),
})

export type LocalModT = z.infer<typeof LocalModSchema>

/**
 * How an import treats what it was given.
 *
 * The one genuinely ambiguous decision: a `.jar` is a zip and must not be
 * unpacked, a Minecraft resource pack is a `.zip` and must not be either, and a
 * mod distributed as a zip of loose files must be. Rust suggests from the
 * game's own install rules; this is what lets the user overrule it.
 */
export const ImportPayloadSchema = z.enum(['file', 'unpack', 'folder'])

export type ImportPayloadT = z.infer<typeof ImportPayloadSchema>

/**
 * One thing waiting to be imported.
 *
 * There is no path here, by construction. `token` is how the file is named back
 * to Rust — see `tmc_core::local::vault` for what that indirection buys and
 * what it does not.
 */
export const PendingFileSchema = z.object({
    token: z.string(),
    name: z.string(),
    isDir: z.boolean(),
    bytes: z.number(),
    isArchive: z.boolean(),
})

export type PendingFileT = z.infer<typeof PendingFileSchema>

export const DropBatchSchema = z.object({
    at: z.number(),
    files: z.array(PendingFileSchema),
})

export type DropBatchT = z.infer<typeof DropBatchSchema>

export const ImportPreviewSchema = z.object({
    token: z.string(),
    name: z.string(),
    isDir: z.boolean(),
    bytes: z.number(),
    payload: ImportPayloadSchema,
    relPath: z.string(),
    source: LocalSourceSchema.nullish(),
    version: z.string().nullish(),
    author: z.string().nullish(),
})

export type ImportPreviewT = z.infer<typeof ImportPreviewSchema>

export const ImportOutcomeSchema = z.object({
    imported: z.array(LocalModSchema),
    /** One line per thing that did not import, naming it. */
    failed: z.array(z.string()),
    sandboxId: z.number().nullable(),
})

export type ImportOutcomeT = z.infer<typeof ImportOutcomeSchema>

/** Something in the game folder the app cannot account for. */
export const AdoptCandidateSchema = z.object({
    token: z.string(),
    name: z.string(),
    relPath: z.string(),
    isDir: z.boolean(),
    files: z.number(),
    bytes: z.number(),
})

export type AdoptCandidateT = z.infer<typeof AdoptCandidateSchema>

/** A mod manager this build can read a library out of. */
export const ManagerInfoSchema = z.object({
    id: z.string(),
    label: z.string(),
    homepage: z.string().nullish(),
    notes: z.array(z.string()),
    /** The TMC games this descriptor can map. */
    slugs: z.array(z.string()),
})

export type ManagerInfoT = z.infer<typeof ManagerInfoSchema>

export const ManagerCandidateSchema = z.object({
    token: z.string(),
    manager: z.string(),
    slug: z.string(),
    gameDir: z.string(),
    /** The profile or instance, when the manager's layout has one. */
    group: z.string().nullish(),
    name: z.string(),
    version: z.string().nullish(),
    author: z.string().nullish(),
    website: z.string().nullish(),
    relPath: z.string().nullish(),
    isDir: z.boolean(),
    files: z.number(),
    bytes: z.number(),
})

export type ManagerCandidateT = z.infer<typeof ManagerCandidateSchema>
