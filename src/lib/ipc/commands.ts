import { z } from 'zod'

import { call } from './index'
import {
    GamePlatformInfoSchema,
    GameStatusSchema,
    GameUpdateReportSchema,
    InstalledGameSchema,
    NativeBuildSchema,
    ApiEnvSchema,
    AutoUpdateReportSchema,
    DependencyReportSchema,
    OutdatedSchema,
    DetectedGameSchema,
    ApplyOutcomeSchema,
    ScanReportSchema,
    SessionSchema,
    SessionRowSchema,
    PlaytimeSummarySchema,
    FlushReportSchema,
    DeployReportSchema,
    PurgeReportSchema,
    QueueSnapshotSchema,
    RconHistorySchema,
    RconProtocolSchema,
    RconReplySchema,
    RconServerSchema,
    LocalModSchema,
    DropBatchSchema,
    ImportPreviewSchema,
    ImportOutcomeSchema,
    AdoptCandidateSchema,
    ManagerInfoSchema,
    ManagerCandidateSchema,
    type ImportPayloadT,
    SandboxModSchema,
    SandboxRowSchema,
    SandboxSpecSchema,
    StageOutcomeSchema,
    QuickInstallReportSchema,
    SharedSandboxSchema,
    ConfigFileSchema,
    SaveReportSchema,
    ImportReportSchema,
    StrategyReportSchema,
    VerifyReportSchema,
    AppSettingsSchema,
    DirListingSchema,
    DirRootSchema,
    InstallOutcomeSchema,
    LaunchPreviewSchema,
    LibraryRowSchema,
    RuleInfoSchema,
    SyncReportSchema,
    LatencySeriesSchema,
    LogEntrySchema,
    LogLevelSchema,
    PendingLoginSchema,
    PingResultSchema,
    PluginPreviewSchema,
    PluginRecordSchema,
    TrustedKeySchema,
    UpdateCheckSchema,
    PollOutcomeSchema,
    QueryOutcomeSchema,
    RunReportSchema,
    SessionSnapshotSchema,
    ThemeSchema,
    type AppSettingsT,
    type LogLevelT,
    type RustQueryProtocolT,
    type WalkLimitsT,
    type FsRootT,
} from './schemas'

/**
 * What Rust needs to query one server.
 *
 * Assembled by the caller from a `ContentSummary`'s `server` block: the address
 * and game port from the server itself, the protocol list and port rules from
 * its game's `query` config. The `id` is opaque — Rust echoes it back so the
 * caller can match results to rows without relying on ordering.
 */
export type QueryRequestT = {
    id: string
    /**
     * Which game this server belongs to.
     *
     * Only used to select a Server Live Query plugin — the built-in protocols
     * come from `protocols` below. Nullable because the view page's
     * single-server call does not always have one to hand.
     */
    appId?: number | null
    host: string
    port: number
    queryPort?: number | null
    protocols?: RustQueryProtocolT[]
    portOffset?: number
    swapGamePort?: boolean
    timeoutMs?: number
    wantPlayers?: boolean
}

/** Every Rust command the app can call, in one place and each parsed. */
export const ipc = {
    // --------------------------------------------------------------- Session
    session: () => call('session_state', SessionSnapshotSchema),
    authBegin: () => call('auth_begin', PendingLoginSchema),
    authPoll: () => call('auth_poll', PollOutcomeSchema),
    authCancel: () => call('auth_cancel', z.void()),
    authSignOut: () => call('auth_sign_out', z.void()),

    /**
     * Which site this build talks to, and whether it is the real one.
     *
     * Read-only by design — there is no `apiSetBase`, and adding one would make
     * the app phishable by anything that can run a line of script in it.
     */
    apiEnv: () => call('api_env', ApiEnvSchema),

    // -------------------------------------------------------------- Settings
    settingsGet: () => call('settings_get', AppSettingsSchema),
    /**
     * Every setting except the two jail-anchor roots.
     *
     * `gameDirs` and `downloadDir` are REFUSED by Rust if they appear here —
     * they anchor the plugin jail rather than describing a preference, so they
     * have their own commands below and their own validation. The type reflects
     * that so the refusal is a compile error rather than a runtime one.
     */
    settingsPatch: (
        patch: Partial<Omit<AppSettingsT, 'gameDirs' | 'downloadDir'>>
    ) => call('settings_patch', AppSettingsSchema, { patch }),

    /**
     * Point a game's install folder somewhere, or `null` to clear it.
     *
     * Rust validates the path before storing it and rejects anything that would
     * make a bad jail anchor — a drive root, a system directory, or any folder
     * containing the app's own data. It stores the CANONICAL path, so the
     * settings this resolves with are the ones the file jail will resolve with.
     */
    settingsSetGameDir: (appId: number | string, dir: string | null) =>
        call('settings_set_game_dir', AppSettingsSchema, {
            appId: String(appId),
            dir,
        }),

    /** Where installers stage downloads, or `null` for the app's own cache. */
    settingsSetDownloadDir: (dir: string | null) =>
        call('settings_set_download_dir', AppSettingsSchema, { dir }),

    settingsReset: () => call('settings_reset', AppSettingsSchema),

    // --------------------------------------------------------------- Logging
    logRead: (limit?: number, minLevel?: LogLevelT) =>
        call('log_read', z.array(LogEntrySchema), {
            limit,
            minLevel: minLevel ? LogLevelSchema.parse(minLevel) : undefined,
        }),
    logClear: () => call('log_clear', z.void()),
    logPath: () => call('log_path', z.string()),

    // --------------------------------------------------------------- Network
    ping: (host: string, port: number, timeoutMs?: number, attempts?: number) =>
        call('ping_server', PingResultSchema, { host, port, timeoutMs, attempts }),

    /**
     * Query a batch of servers with their game's own protocol.
     *
     * ONE call for the whole viewport. Fifty per-card calls would be fifty IPC
     * round trips per refresh, and the concurrency cap has to be applied
     * somewhere that can see the whole batch.
     */
    queryServers: (requests: QueryRequestT[]) =>
        call('query_servers', z.array(QueryOutcomeSchema), { requests }),

    /** One server, in full — the view page's live panel, with the roster. */
    queryServer: (request: QueryRequestT) =>
        call('query_server', QueryOutcomeSchema, { request }),

    latencySeries: (keys: string[]) =>
        call('latency_series', z.array(LatencySeriesSchema), { keys }),

    latencyClear: () => call('latency_clear', z.void()),

    // --------------------------------------------------------- Folder picker
    /**
     * The places the picker offers to start from — home, downloads, and every
     * drive letter that answers on Windows.
     */
    fsRoots: () => call('fs_roots', z.array(DirRootSchema)),

    /**
     * The directories directly inside `path`, for the app's own folder picker.
     *
     * Directories only — no file is ever named in the reply. See the module
     * header on `commands/fs.rs` for what this widens and what it does not.
     */
    fsListDirs: (path: string | null, showHidden = false) =>
        call('fs_list_dirs', DirListingSchema, {
            request: { path, showHidden },
        }),

    // --------------------------------------------------------------- Plugins
    pluginList: () => call('plugin_list', z.array(PluginRecordSchema)),
    pluginInspect: (dir: string) =>
        call('plugin_inspect', PluginPreviewSchema, { dir }),

    /**
     * Ask the site whether this build is out of date.
     *
     * Answers with a version and a link, never with an artifact: the app does
     * not update itself. See `commands/api.rs`.
     */
    updateCheck: () => call('update_check', UpdateCheckSchema),

    /** Every publishing key the user trusts. */
    pluginTrustedKeys: () => call('plugin_trusted_keys', z.array(TrustedKeySchema)),
    /**
     * Trust a key.
     *
     * Widens what may run when `requireSignedPlugins` is on, so it is audited
     * at Security level in Rust — a change to that is visible in the log
     * whether or not logging is turned up.
     */
    pluginTrustKey: (id: string, label: string, publicKey: string) =>
        call('plugin_trust_key', TrustedKeySchema, { id, label, publicKey }),
    pluginUntrustKey: (id: string) =>
        call('plugin_untrust_key', z.boolean(), { id }),
    /** `fingerprint` must be the one `pluginInspect` returned — see the Rust side. */
    pluginApprove: (dir: string, fingerprint: string) =>
        call('plugin_approve', PluginRecordSchema, { dir, fingerprint }),
    pluginSetEnabled: (id: string, enabled: boolean) =>
        call('plugin_set_enabled', z.void(), { id, enabled }),
    pluginRemove: (id: string) => call('plugin_remove', z.void(), { id }),
    pluginTheme: (id: string) =>
        call('plugin_theme', ThemeSchema.nullable(), { id }),

    pluginRun: (request: {
        plugin: string
        action: 'install' | 'uninstall'
        appId?: number
        context?: Record<string, string>
    }) => call('plugin_run', RunReportSchema, { request }),

    pluginQueryServer: (
        plugin: string,
        host: string,
        port: number,
        queryPort?: number
    ) =>
        call('plugin_query_server', z.record(z.string(), z.unknown()), {
            plugin,
            host,
            port,
            queryPort,
        }),

    // --------------------------------------------------------------- Library
    libraryList: () => call('library_list', z.array(LibraryRowSchema)),

    /**
     * One sync pass, which also ACTS on its plan.
     *
     * The installs it queues are run in Rust, not here. A frontend that decided
     * what to install would be a frontend an injected script could talk into
     * installing something — and the whole architecture rests on the webview
     * being unable to reach the filesystem.
     */
    librarySync: (forceFull?: boolean) =>
        call('library_sync', SyncReportSchema, { forceFull }),

    libraryInstall: (id: string) =>
        call('library_install', InstallOutcomeSchema, { id }),
    libraryUninstall: (id: string) =>
        call('library_uninstall', InstallOutcomeSchema, { id }),
    /**
     * The cloud installs this device has mirrored, as raw payloads.
     *
     * Parsed with the API contract's own `InstallSchema` by the caller rather
     * than re-declared here: the shape is the contract's, and a third copy in
     * `schemas.ts` would be the one most likely to drift.
     */
    libraryInstalls: () =>
        call('library_installs', z.array(z.record(z.string(), z.unknown()))),

    /** Every install's folder on this machine, as `[id, dir]` pairs. */
    libraryInstallDirs: () =>
        call(
            'library_install_dirs',
            z.array(z.tuple([z.number(), z.string().nullable()]))
        ),

    /** The folder on THIS machine an install materialises into. */
    librarySetInstallDir: (id: number, dir: string | null) =>
        call('library_set_install_dir', z.void(), { id, dir }),

    librarySupportedApps: () => call('library_supported_apps', z.array(z.string())),
    libraryPluginErrors: () =>
        call('library_plugin_errors', z.array(z.tuple([z.string(), z.string()]))),
    libraryReloadPlugins: () => call('library_reload_plugins', z.number()),
    libraryRulesFor: (slug: string) =>
        call('library_rules_for', z.array(RuleInfoSchema), { slug }),

    // ---------------------------------------------------------------- Launch
    /** Resolve what launching would run, without running it. */
    launchPreview: (installId: number) =>
        call('launch_preview', LaunchPreviewSchema, { installId }),
    launchInstall: (installId: number) =>
        call('launch_install', LaunchPreviewSchema, { installId }),
    launchAvailable: (slug: string) =>
        call('launch_available', z.boolean(), { slug }),

    // -------------------------------------------------------------- Sandboxes
    //
    // A sandbox is named by ID everywhere below, never by a directory. Rust
    // looks up where it deploys; a command that took a target path would make
    // every check in the anchor validator and the deployment engine advisory.

    sandboxList: (appId?: number) =>
        call('sandbox_list', z.array(SandboxRowSchema), { appId }),

    sandboxGet: (id: number) =>
        call('sandbox_get', SandboxRowSchema.nullable(), { id }),

    /**
     * Create one, optionally from one of the game's presets.
     *
     * The preset is applied in RUST, not here: it carries a deployment strategy
     * and a set of option values, and assembling those in the webview would
     * mean a sandbox whose settings never went through the game's own schema.
     */
    sandboxCreate: (
        sandbox: {
            appId: number
            name: string
            appSlug?: string | null
            appName?: string | null
            description?: string | null
            environment?: 'client' | 'server' | 'shared'
            strategy?: 'direct' | 'hardlink' | 'symlink' | 'usvfs'
            gameVersion?: string | null
            loader?: string | null
            gameDir?: string | null
            options?: Record<string, unknown>
            cloudSync?: boolean
            autoUpdate?: boolean
        },
        preset?: string
    ) => call('sandbox_create', SandboxRowSchema, { new: sandbox, preset }),

    sandboxPatch: (
        id: number,
        patch: {
            name?: string
            description?: string | null
            environment?: 'client' | 'server' | 'shared'
            strategy?: 'direct' | 'hardlink' | 'symlink' | 'usvfs'
            gameVersion?: string | null
            loader?: string | null
            gameDir?: string | null
            options?: Record<string, unknown>
            launchArgs?: string[]
            launchEnv?: Record<string, string>
            cloudSync?: boolean
            autoUpdate?: boolean
            isDefault?: boolean
        }
    ) => call('sandbox_patch', SandboxRowSchema, { id, patch }),

    /**
     * Turn a sandbox into a code somebody can paste.
     *
     * A code, not a file. A file export needs a path to write to and a file
     * import needs a path to read from, and the webview names neither — trading
     * that guarantee for a save dialog is not a trade worth making. It is also
     * what people actually do with an exported profile: put it in a message.
     *
     * It carries item IDS, never files. Shipping the files would make this a
     * redistribution channel for other people's work, with no download counted,
     * no licence respected and no update path.
     */
    sandboxExport: (id: number) => call('sandbox_export', z.string(), { id }),

    /** What importing a code would do, without doing any of it. */
    sandboxImportPreview: (code: string) =>
        call('sandbox_import_preview', SharedSandboxSchema, { code }),

    /**
     * Create a sandbox from a code.
     *
     * Subscribes each item on the account and adds it; it does NOT stage. Two
     * hundred mods is two hundred downloads, and starting them inside a command
     * the UI is awaiting is a frozen dialog with no queue to look at.
     */
    sandboxImport: (code: string, name?: string) =>
        call('sandbox_import', ImportReportSchema, { code, name }),

    /** Undeploys first — the ledger goes with the row, so it has to. */
    sandboxDelete: (id: number, keepFiles = false) =>
        call('sandbox_delete', PurgeReportSchema, { id, keepFiles }),

    sandboxSetDefault: (id: number) =>
        call('sandbox_set_default', z.void(), { id }),

    sandboxAddMod: (id: number, kind: string, itemId: number) =>
        call('sandbox_add_mod', SandboxRowSchema, { id, kind, itemId }),

    /**
     * Install one mod or asset into one sandbox, in a single operation.
     *
     * Subscribe if needed, sync, add, stage, deploy — as ONE Rust command
     * rather than five calls from here. The plan is executed in Rust because a
     * frontend that orchestrates it is a frontend an injected script can run
     * four fifths of, leaving a game folder half-modded with nothing on screen
     * saying so.
     *
     * A mod and an asset take the identical path: they differ only in which app
     * rule matches them, and that selection already happens inside the executor.
     */
    sandboxInstallItem: (id: number, kind: string, itemId: number, deploy = true) =>
        call('sandbox_install_item', QuickInstallReportSchema, {
            id,
            kind,
            itemId,
            deploy,
        }),

    sandboxRemoveMod: (id: number, modKey: string) =>
        call('sandbox_remove_mod', SandboxRowSchema, { id, modKey }),

    sandboxSetModEnabled: (id: number, modKey: string, enabled: boolean) =>
        call('sandbox_set_mod_enabled', z.void(), { id, modKey, enabled }),

    /** The load order, first to last. Unnamed entries keep their place after. */
    sandboxReorder: (id: number, keys: string[]) =>
        call('sandbox_reorder', z.array(SandboxModSchema), { id, keys }),

    /** Download and unpack whatever is not staged yet. */
    sandboxStage: (id: number, force = false) =>
        call('sandbox_stage', z.array(StageOutcomeSchema), { id, force }),

    sandboxDeploy: (id: number, dryRun = false) =>
        call('sandbox_deploy', DeployReportSchema, { id, dryRun }),

    sandboxPurge: (id: number) => call('sandbox_purge', PurgeReportSchema, { id }),

    sandboxVerify: (id: number) =>
        call('sandbox_verify', VerifyReportSchema, { id }),

    /** Which strategies would actually work here, and why the others would not. */
    sandboxStrategies: (id: number) =>
        call('sandbox_strategies', z.array(StrategyReportSchema), { id }),

    /** What starting this sandbox would run, without starting it. */
    sandboxLaunchPreview: (id: number) =>
        call('sandbox_launch_preview', LaunchPreviewSchema, { id }),
    sandboxLaunch: (id: number) =>
        call('sandbox_launch', LaunchPreviewSchema, { id }),

    /** A game's presets, option schema and deployment rules, or null. */
    sandboxSpec: (slug: string) =>
        call('sandbox_spec', SandboxSpecSchema.nullable(), { slug }),

    /**
     * What this sandbox's items say about each other.
     *
     * A LOCAL query — every edge was cached when its item was added — so a
     * screen can call it on every render without a request going anywhere.
     */
    sandboxCheck: (id: number) =>
        call('sandbox_check', DependencyReportSchema, { id }),

    /**
     * Everything with a newer release than the one staged.
     *
     * Ignores the auto-update switches: somebody who turned them off still
     * wants to be told, and being told is the point of turning them off rather
     * than unsubscribing.
     */
    sandboxUpdates: () => call('sandbox_updates', z.array(OutdatedSchema)),

    /**
     * Bring every eligible sandbox forward.
     *
     * Redeploys only the sandboxes that were ALREADY deployed — staging is
     * invisible, deploying writes into somebody's game folder, and an automatic
     * pass may not do the second on its own initiative.
     */
    sandboxAutoUpdate: () => call('sandbox_auto_update', AutoUpdateReportSchema),

    /** Re-fetch every member's edges. What "check again" calls. */
    sandboxRefreshDependencies: (id: number) =>
        call('sandbox_refresh_dependencies', DependencyReportSchema, { id }),

    /**
     * Add everything missing that the account is already subscribed to.
     *
     * Only subscribed items: the installer refuses one the account did not ask
     * to keep, so adding the row would produce an entry that can never
     * download. Returns the names it added.
     */
    sandboxAddMissing: (id: number) =>
        call('sandbox_add_missing', z.array(z.string()), { id }),

    // -------------------------------------------------------------- Downloads
    //
    // There is deliberately no `downloadStart(url, path)`: it would be an
    // arbitrary-write primitive with a progress bar attached. Everything here
    // acts on a queue row that already exists.

    downloadList: () => call('download_list', QueueSnapshotSchema),
    downloadPause: (id: string) => call('download_pause', z.void(), { id }),
    downloadResume: (id: string) => call('download_resume', z.void(), { id }),
    downloadCancel: (id: string) => call('download_cancel', z.void(), { id }),

    downloadSetPriority: (id: string, priority: number) =>
        call('download_set_priority', z.void(), { id, priority }),

    /** Bytes per second, or null for no limit. */
    downloadSetLimit: (id: string, bps: number | null) =>
        call('download_set_limit', z.void(), { id, bps }),

    downloadSetGlobalLimit: (bps: number | null) =>
        call('download_set_global_limit', z.void(), { bps }),

    downloadSetConcurrency: (n: number) =>
        call('download_set_concurrency', z.void(), { n }),

    /**
     * Queue one specific release of one item.
     *
     * Three ids and a kind — no URL and no destination. Rust looks the release
     * up through the API and puts the file in the user's download folder, which
     * is what keeps this inside the rule that no command here takes either.
     */
    downloadRelease: (kind: string, itemId: number, releaseId: number) =>
        call('download_release', z.string(), { kind, itemId, releaseId }),

    downloadClearFinished: () => call('download_clear_finished', z.number()),

    // -------------------------------------------------------------- Detection
    /** Read-only. Applying a result is a separate, deliberate step. */
    detectGames: () => call('detect_games', z.array(DetectedGameSchema)),

    /**
     * Point a game's folder at a detected one.
     *
     * Takes the SLUG, not an app id — Rust resolves the id itself, so a
     * mismatched pair cannot point one game's installer at another's folder.
     */
    detectApply: (slug: string, path: string) =>
        call('detect_apply', z.void(), { slug, path }),

    /**
     * Walk folders the user ticked, for what the launchers do not know about.
     *
     * `roots` is required and has no default. A scan of "the filesystem" is a
     * scan of somebody's documents and every network share they have mounted,
     * and the app has no business enumerating any of it — so the folders come
     * from a person, one tick at a time. Rust bounds depth, directory count and
     * wall clock on top of that.
     */
    detectScan: (roots: string[], limits?: WalkLimitsT) =>
        call('detect_scan', ScanReportSchema, { request: { roots, limits } }),

    /**
     * Ask a running scan to stop.
     *
     * Takes effect between two directories rather than immediately, which on a
     * dead network share can still be several seconds — so the UI says
     * "stopping" rather than closing itself.
     */
    detectScanCancel: () => call('detect_scan_cancel', z.void()),

    detectScanRunning: () => call('detect_scan_running', z.boolean()),

    /**
     * Apply several results at once.
     *
     * Each goes through `detect_apply`'s own validation and a failure does not
     * stop the rest — a batch where one folder is refused must still apply the
     * other eleven and say which one was not.
     */
    detectApplyMany: (games: [slug: string, path: string][]) =>
        call('detect_apply_many', z.array(ApplyOutcomeSchema), { games }),

    // --------------------------------------------------- Game settings files
    //
    // The webview names a file, which is the one thing the rest of this surface
    // avoids — so it is bounded twice: the GAME declares which files exist
    // (`plugins/app/<slug>/config.json`), and Rust re-runs those rules on every
    // read and write on top of the jail. `allowConfigEditing` removes all three
    // commands.

    /** Does this sandbox's game describe where it keeps its settings? */
    configAvailable: (sandboxId: number) =>
        call('config_available', z.boolean(), { sandboxId }),

    configList: (sandboxId: number) =>
        call('config_list', z.array(ConfigFileSchema), { sandboxId }),

    configRead: (sandboxId: number, root: FsRootT, path: string) =>
        call('config_read', z.string(), { sandboxId, root, path }),

    configWrite: (
        sandboxId: number,
        root: FsRootT,
        path: string,
        contents: string
    ) =>
        call('config_write', SaveReportSchema, {
            sandboxId,
            root,
            path,
            contents,
        }),

    // ------------------------------------------------------------------ Play
    //
    // The webview names IDS. It never names a loader URL: that string becomes a
    // `<script src>` on the SITE's origin, so Rust resolves the launch through
    // `/play/launch` itself — the same rule `downloadRelease` follows.

    /**
     * Open the game in a window of its own.
     *
     * The window is a REMOTE page, which is the isolation the feature rests on:
     * Tauri exposes commands to the app's own origin only, so a loader running
     * there has no `invoke` at all. See `commands/play.rs`.
     */
    playOpenWeb: (request: {
        appId: number
        serverId?: number
        options?: Record<string, string | number | boolean>
        title?: string
        appSlug?: string
        /**
         * Open it filling the screen.
         *
         * Chosen before the window opens because the window cannot choose it
         * afterwards: it is a remote page with no `invoke`, which is the
         * isolation the whole feature rests on.
         */
        fullscreen?: boolean
    }) => call('play_open_web', SessionSchema, { request }),

    /** Resolve the site's `playAppUri` and hand it to the OS. */
    playHandoff: (request: {
        appId: number
        serverId?: number
        options?: Record<string, string | number | boolean>
        title?: string
        appSlug?: string
    }) => call('play_handoff', SessionSchema, { request }),

    /**
     * Hand a server's own connect link to the installed game client.
     *
     * Takes a server ID, never a URL: "open this URI" hands a string to
     * whatever program claimed a scheme, so Rust reads the link from the API
     * and checks it against the same closed scheme list a plugin's launch rule
     * is held to. It cannot go through `openUrl` — that capability is scoped to
     * `https://*`, which is exactly what keeps the webview from opening a
     * `steam://` link on its own.
     */
    playConnect: (serverId: number) =>
        call('play_connect', SessionSchema, { serverId }),

    playClose: () => call('play_close', z.void()),

    playState: () => call('play_state', z.object({ open: z.boolean() })),

    // ------------------------------------------------------------- TMC games
    //
    // Installing a game TMC publishes. The webview names an APP ID and nothing
    // else: the build's address, its checksum and the path it unpacks to are
    // all resolved in Rust, so there is no `gameInstall(url)` and no
    // `gameLaunch(path)` — the same rule `downloadRelease` and `playOpenWeb`
    // already follow.

    /** What this machine can install, and why it cannot when it cannot. */
    gamesPlatform: () => call('games_platform', GamePlatformInfoSchema),

    /** Every TMC game installed on this device. */
    gamesList: () => call('games_list', z.array(InstalledGameSchema)),

    /** What the site has for this machine, or null for nothing to install. */
    gamesAvailable: (appId: number) =>
        call('games_available', NativeBuildSchema.nullable(), { appId }),

    /**
     * Install, or update in place.
     *
     * One command for both, because an update is an install whose row already
     * exists. Progress is NOT reported through here — the build goes through
     * the ordinary download queue, so the Downloads screen shows it, pauses it
     * and rate-limits it without learning what a game is.
     */
    gameInstall: (request: { appId: number; name: string; slug?: string }) =>
        call('game_install', InstalledGameSchema, { request }),

    gameUninstall: (appId: number) => call('game_uninstall', z.void(), { appId }),

    gameSetAutoUpdate: (appId: number, enabled: boolean) =>
        call('game_set_auto_update', z.void(), { appId, enabled }),

    /** Ask the site about every installed game. Reports; installs nothing. */
    gamesCheckUpdates: () => call('games_check_updates', z.array(GameStatusSchema)),

    /**
     * Bring every installed game that asked to be kept current forward.
     *
     * Called on the library sync pass, beside `sandboxAutoUpdate` and for the
     * same reason: that loop is the device's own "am I online now?" heartbeat.
     * A game that fails keeps the version it already had — the new build lands
     * in its own directory and the row is only repointed once it is verified.
     */
    gamesAutoUpdate: () => call('games_auto_update', GameUpdateReportSchema),

    /**
     * Start an installed game, optionally into a server.
     *
     * Names a SERVER ID, never an address: the host and port are read from the
     * API in Rust, for the same reason `playConnect` reads the connect link
     * there. Unlike a `steam://` hand-off this is a real process — it has a
     * duration, an exit code and a captured log.
     */
    gameLaunch: (request: {
        appId: number
        serverId?: number
        options?: Record<string, string | number | boolean>
        locale?: string
    }) => call('game_launch', SessionSchema, { request }),

    // -------------------------------------------------------------- Sessions
    //
    // Nothing here STARTS anything. A launch goes through `launchInstall` or
    // `sandboxLaunch`, which are the two places holding a resolved plan and the
    // checks that produced it.

    /** What is running right now. */
    sessionRunning: () => call('session_running', z.array(SessionSchema)),

    /** Recent launches, from the durable history — survives a restart. */
    sessionHistory: (appId?: number, limit?: number) =>
        call('session_history', z.array(SessionRowSchema), { appId, limit }),

    /** Kill a running game. See `Sessions::stop` for why there is no polite ask. */
    sessionStop: (id: number) => call('session_stop', z.void(), { id }),

    /** The last lines a session's process printed. The crash report. */
    sessionLog: (id: number, lines?: number) =>
        call('session_log', z.string(), { id, lines }),

    playtimeSummary: () => call('playtime_summary', PlaytimeSummarySchema),

    /**
     * Send outstanding playtime to the account.
     *
     * A game is very often played offline, so a session is written first and
     * reported whenever the device next has an API to talk to.
     */
    sessionsFlush: () => call('sessions_flush', FlushReportSchema),

    // ------------------------------------------------------------------ RCON
    //
    // No command here returns a password, and there is no shape one could take
    // that would: a password goes in on create or change, and after that the
    // webview can only name a server id.

    rconList: () => call('rcon_list', z.array(RconServerSchema)),

    rconCreate: (server: {
        name: string
        host: string
        port: number
        protocol?: 'source' | 'frostbite'
        password: string
        serverId?: number | null
        appId?: number | null
    }) => call('rcon_create', z.number(), { server }),

    rconUpdate: (id: number, name: string, host: string, port: number) =>
        call('rcon_update', z.void(), { id, name, host, port }),

    rconSetPassword: (id: number, password: string | null) =>
        call('rcon_set_password', z.void(), { id, password }),

    rconDelete: (id: number) => call('rcon_delete', z.void(), { id }),

    /** Opens a session, so a wrong password is reported before a command is. */
    rconConnect: (id: number) => call('rcon_connect', z.void(), { id }),
    rconDisconnect: (id: number) => call('rcon_disconnect', z.void(), { id }),
    rconIsConnected: (id: number) => call('rcon_is_connected', z.boolean(), { id }),

    rconExec: (id: number, command: string, timeoutMs?: number) =>
        call('rcon_exec', RconReplySchema, { id, command, timeoutMs }),

    rconHistory: (id: number, limit?: number) =>
        call('rcon_history', z.array(RconHistorySchema), { id, limit }),

    rconClearHistory: (id: number) => call('rcon_clear_history', z.void(), { id }),

    rconSuggestProtocol: (queryProtocol: string | null) =>
        call('rcon_suggest_protocol', RconProtocolSchema, { queryProtocol }),

    // ---------------------------------------------------- Imported content
    //
    // Mods with no account behind them: dropped files, folders the game already
    // had, other managers' libraries.
    //
    // Nothing here takes a filesystem path. Rust found every path these act on
    // — the OS delivered a drop, or one of the two scans listed a directory —
    // and hands back an opaque token; the webview says WHICH of the things you
    // found, never THIS path. `tmc_core::local::vault` is where that is
    // written down, including what it does not buy.

    /**
     * The files from the most recent drop on the window.
     *
     * A poll as well as an event, because the event fires whether or not
     * anything is listening — a drop that lands during a route change would
     * otherwise be silently lost, which reads as drag and drop being broken
     * rather than as a race. Taking it CLEARS it, so a remount does not
     * re-open a dialog for files already dealt with.
     */
    importDropped: () => call('import_dropped', DropBatchSchema),

    /** What importing these would do, without doing any of it. */
    importPreview: (tokens: string[], appId?: number) =>
        call('import_preview', z.array(ImportPreviewSchema), { tokens, appId }),

    /**
     * Import them into the device's store.
     *
     * `sandboxId` also puts each one in that sandbox, which is what dropping
     * onto a sandbox means. Nothing is deployed either way — that is a separate
     * click through the same engine every subscribed mod goes through.
     */
    importPaths: (
        tokens: string[],
        opts?: {
            appId?: number
            sandboxId?: number
            origin?: 'dropped' | 'adopted' | 'manager'
            overrides?: {
                token: string
                name?: string
                relPath?: string
                payload?: ImportPayloadT
            }[]
        }
    ) =>
        call('import_paths', ImportOutcomeSchema, {
            tokens,
            appId: opts?.appId,
            sandboxId: opts?.sandboxId,
            origin: opts?.origin,
            overrides: opts?.overrides,
        }),

    /**
     * What is already in this sandbox's game folder that nothing here claims.
     *
     * Reads only the folders the game's own rules declare as mod targets, and
     * subtracts every file the deployment ledger, an installed subscription or
     * a previous adoption accounts for. Offering something the app already
     * deployed would list one mod twice and make every one of its files
     * conflict with itself.
     */
    importUnmanaged: (sandboxId: number) =>
        call('import_unmanaged', z.array(AdoptCandidateSchema), { sandboxId }),

    /** The mod managers this build can read a library out of. */
    importManagers: () => call('import_managers', z.array(ManagerInfoSchema)),

    /**
     * Read one manager's own storage.
     *
     * Nothing there is modified, then or later: importing a result COPIES it,
     * so going back to that manager tomorrow finds everything where it was.
     */
    importManagerScan: (id: string) =>
        call('import_manager_scan', z.array(ManagerCandidateSchema), { id }),

    localList: (appId?: number) =>
        call('local_list', z.array(LocalModSchema), { appId }),

    localGet: (id: number) => call('local_get', LocalModSchema.nullable(), { id }),

    /**
     * Rename it, re-version it, note something, or move where it lands.
     *
     * Changing `relPath` MOVES the files inside the app's store, which is why
     * it is not a column write: a row that says `BepInEx/plugins` while its
     * files sit under `mods` deploys to the wrong place and nothing downstream
     * would notice. The sandboxes holding it need a redeploy afterwards.
     */
    localPatch: (
        id: number,
        patch: {
            name?: string
            version?: string
            author?: string
            notes?: string
            appId?: number
            appSlug?: string
            relPath?: string
        }
    ) => call('local_patch', LocalModSchema, { id, patch }),

    /**
     * Forget it and delete its files.
     *
     * Returns the sandboxes that held it — they now have files in a game folder
     * that only a redeploy will take out, and saying so is the caller's job.
     */
    localDelete: (id: number) => call('local_delete', z.array(z.number()), { id }),

    localAddToSandbox: (sandboxId: number, id: number) =>
        call('local_add_to_sandbox', z.string(), { sandboxId, id }),
}
