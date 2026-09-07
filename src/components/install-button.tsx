import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { FiAlertTriangle, FiCheck, FiDownloadCloud } from 'react-icons/fi'
import { Button } from '@modcommunity/shared'

import { ipc } from '~/lib/ipc/commands'
import { isIpcError } from '~/lib/ipc'
import type { RunReportT } from '~/lib/ipc/schemas'
import type { ContentSummaryT, ReleaseT } from '~/lib/api/contract'
import { useSettings } from '~/lib/settings/provider'

/**
 * One-click install, when a plugin exists for the game.
 *
 * The app itself knows nothing about how any game loads a mod — that is the
 * whole reason installer plugins exist. This button's only job is matching an
 * item to an approved plugin for its `app`, showing the user exactly what will
 * run, and reporting the result.
 *
 * It stays absent rather than disabled when no plugin matches. A greyed-out
 * install button on every mod trains people to ignore it; a pointer to the
 * plugin manager is the actionable version.
 */
export default function InstallButton({
    summary,
    releases,
}: {
    summary: ContentSummaryT
    releases: ReleaseT[]
}) {
    const { app } = useSettings()

    const [confirming, setConfirming] = useState(false)
    const [running, setRunning] = useState(false)
    const [report, setReport] = useState<RunReportT | null>(null)
    const [error, setError] = useState<string | null>(null)

    const plugins = useQuery({
        queryKey: ['plugins'],
        queryFn: () => ipc.pluginList(),
        staleTime: 30 * 1000,
    })

    const appId = summary.app?.id

    const match = plugins.data?.find(
        (plugin) =>
            plugin.enabled &&
            !plugin.needsReapproval &&
            plugin.kinds.includes('installer') &&
            (plugin.apps.length === 0 ||
                (appId != null && plugin.apps.includes(appId)))
    )

    if (!match)
        return (
            <Link
                to="/settings/plugins"
                className="flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-xs text-muted"
            >
                <FiDownloadCloud className="size-3.5" />
                No installer for {summary.app?.name ?? 'this game'}
            </Link>
        )

    const gameDir = appId != null ? app?.gameDirs[String(appId)] : undefined
    const file = releases[0]?.files[0]

    const run = async () => {
        setRunning(true)
        setError(null)
        setReport(null)

        try {
            const result = await ipc.pluginRun({
                plugin: match.id,
                action: 'install',
                appId,
                /*
                 * Context values are substituted literally into the plan's
                 * `{name}` placeholders. Only facts about the item go in here —
                 * never a path, never anything from settings, since the plan's
                 * paths are resolved by the plugin file jail and must not be
                 * influenceable from this side.
                 */
                context: {
                    id: summary.id,
                    kind: summary.kind,
                    name: summary.name,
                    version: releases[0]?.version ?? '',
                    fileUrl: file?.url ?? '',
                    fileName: file?.title ?? `${summary.id}.zip`,
                },
            })

            setReport(result)
        } catch (err) {
            setError(isIpcError(err) ? err.message : 'The install failed.')
        } finally {
            setRunning(false)
            setConfirming(false)
        }
    }

    if (report)
        return (
            <div
                className={`flex items-start gap-2 rounded-lg border px-3 py-2 text-xs ${
                    report.ok
                        ? 'border-success text-success'
                        : 'border-danger text-danger'
                }`}
            >
                {report.ok ? (
                    <FiCheck className="mt-0.5 size-3.5 shrink-0" />
                ) : (
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                )}
                <span>
                    {report.ok
                        ? `Installed (${report.stepsRun} steps).`
                        : (report.error ?? 'The install failed.')}{' '}
                    <Link to="/settings/logging" className="underline">
                        View the log
                    </Link>
                </span>
            </div>
        )

    if (confirming)
        return (
            <div className="flex w-full flex-col gap-2 rounded-lg border border-border bg-surface p-3 text-xs">
                <p className="font-medium">Install with “{match.name}”?</p>

                <ul className="flex list-disc flex-col gap-0.5 pl-4 text-muted">
                    {match.permissions.map((permission) => (
                        <li key={permission}>{permission}</li>
                    ))}
                </ul>

                {!gameDir && (
                    <p className="text-danger">
                        No install folder is set for{' '}
                        {summary.app?.name ?? 'this game'}.{' '}
                        <Link to="/settings/games" className="underline">
                            Set one first
                        </Link>
                        .
                    </p>
                )}

                {error && <p className="text-danger">{error}</p>}

                <div className="flex gap-2">
                    <Button
                        btnType="primary"
                        disabled={running || !gameDir}
                        onClick={() => void run()}
                    >
                        {running ? 'Installing…' : 'Install'}
                    </Button>
                    <Button
                        btnType="secondary"
                        disabled={running}
                        onClick={() => setConfirming(false)}
                    >
                        Cancel
                    </Button>
                </div>
            </div>
        )

    return (
        <Button
            btnType="primary"
            onClick={() => {
                // `confirmEveryRun` defaults ON. Turning it off is a deliberate
                // choice by someone installing dozens of mods; the plugin's
                // permissions were still shown once, at approval.
                if (app?.confirmEveryRun === false) void run()
                else setConfirming(true)
            }}
        >
            <span className="flex items-center gap-2">
                <FiDownloadCloud className="size-4" /> Install
            </span>
        </Button>
    )
}
