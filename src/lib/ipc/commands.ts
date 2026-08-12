import { z } from 'zod'

import { call } from './index'
import {
    ApiEnvSchema,
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
    PollOutcomeSchema,
    QueryOutcomeSchema,
    RunReportSchema,
    SessionSnapshotSchema,
    ThemeSchema,
    type AppSettingsT,
    type LogLevelT,
    type RustQueryProtocolT,
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
     * Every setting except the two sandbox roots.
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
     * settings this resolves with are the ones the sandbox will resolve with.
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
}
