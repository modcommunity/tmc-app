import { z } from 'zod'

import { call } from './index'
import {
    AppSettingsSchema,
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

    // -------------------------------------------------------------- Settings
    settingsGet: () => call('settings_get', AppSettingsSchema),
    settingsPatch: (patch: Partial<AppSettingsT>) =>
        call('settings_patch', AppSettingsSchema, { patch }),
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

    // --------------------------------------------------------------- Plugins
    pluginList: () => call('plugin_list', z.array(PluginRecordSchema)),
    pluginInspect: (dir: string) => call('plugin_inspect', PluginPreviewSchema, { dir }),
    /** `fingerprint` must be the one `pluginInspect` returned — see the Rust side. */
    pluginApprove: (dir: string, fingerprint: string) =>
        call('plugin_approve', PluginRecordSchema, { dir, fingerprint }),
    pluginSetEnabled: (id: string, enabled: boolean) =>
        call('plugin_set_enabled', z.void(), { id, enabled }),
    pluginRemove: (id: string) => call('plugin_remove', z.void(), { id }),
    pluginTheme: (id: string) => call('plugin_theme', ThemeSchema.nullable(), { id }),

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
}
