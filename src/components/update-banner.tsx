/**
 * "There is a newer version" — and nothing more than that.
 *
 * The app does not update itself. This asks the site what the newest published
 * version is, compares it against the running build in Rust, and if it is
 * behind, offers a link that opens the download page in the user's real
 * browser.
 *
 * WHY NOT A REAL UPDATER
 * ----------------------
 * `tauri-plugin-updater` needs a signing key held by whoever cuts releases and
 * a manifest endpoint to serve. Neither exists, and configuring the client half
 * against neither would be a feature that names a capability it does not have —
 * with a silently-installed binary as the consequence rather than a stale
 * banner. When those exist this becomes the fallback for platforms the updater
 * does not cover; it does not become dead code.
 *
 * WHEN IT ASKS
 * ------------
 * Once per launch, and only when `autoUpdateCheck` is on. Not on a timer: an
 * app left open for a week does not need to be told about a release nine times,
 * and the check is one request whose answer changes on the scale of weeks.
 *
 * Failure is SILENT. Somebody offline, or behind a captive portal, or on a
 * self-hosted base with no version setting configured, is not a person who
 * needs an error about the update check — they did not ask for it, and the
 * thing it would interrupt is whatever they actually opened the app to do.
 */

import { useEffect, useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { FiDownload, FiX } from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { useSettings } from '~/lib/settings/provider'
import type { UpdateCheckT } from '~/lib/ipc/schemas'

/**
 * Versions the user has dismissed, so a notice stays dismissed across launches.
 *
 * Keyed by the version rather than a boolean: dismissing 1.4 must not hide 1.5.
 */
const DISMISSED_KEY = 'tmc.update-dismissed'

export default function UpdateBanner() {
    const { app } = useSettings()

    const [found, setFound] = useState<UpdateCheckT | null>(null)
    const [dismissed, setDismissed] = useState<string | null>(() => read())

    const enabled = app?.autoUpdateCheck ?? false

    useEffect(() => {
        if (!enabled) return

        let live = true

        void (async () => {
            try {
                const result = await ipc.updateCheck()

                if (live && result.outdated) setFound(result)
            } catch {
                // Silent, deliberately — see the module header.
            }
        })()

        return () => {
            live = false
        }
        // Once per launch. `enabled` is the only thing that should re-trigger
        // it, and it changes only when the user toggles the setting.
    }, [enabled])

    if (!found?.latest) return null
    if (dismissed === found.latest) return null

    return (
        <div className="flex items-center gap-3 border-b border-border bg-surface-2 px-3 py-2 text-xs">
            <FiDownload aria-hidden className="size-4 shrink-0 text-accent" />

            <p className="min-w-0 flex-1">
                Version {found.latest} is available. You have {found.current}.
            </p>

            {found.download && (
                <button
                    type="button"
                    onClick={() => void openUrl(found.download!)}
                    className="shrink-0 rounded-lg bg-accent px-2.5 py-1 font-semibold text-accent-foreground"
                >
                    Get it
                </button>
            )}

            <button
                type="button"
                aria-label="Dismiss this update notice"
                onClick={() => {
                    if (found.latest) remember(found.latest)

                    setDismissed(found.latest)
                }}
                className="shrink-0 rounded-lg border border-border p-1 text-muted"
            >
                <FiX className="size-3.5" />
            </button>
        </div>
    )
}

function read(): string | null {
    try {
        return window.localStorage.getItem(DISMISSED_KEY)
    } catch {
        return null
    }
}

function remember(version: string): void {
    try {
        window.localStorage.setItem(DISMISSED_KEY, version)
    } catch {
        // A webview with storage disabled. The notice comes back next launch,
        // which is a nuisance rather than a fault.
    }
}
