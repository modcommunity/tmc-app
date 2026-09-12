/**
 * "There is a newer version" — and nothing more than that.
 *
 * The app does not update itself. This asks the site what the newest published
 * version is, compares it against the running build in Rust, and if it is
 * behind, offers a link that opens the download page in the user's real
 * browser.
 *
 * WHEN IT CAN DO MORE THAN THAT
 * -----------------------------
 * A build compiled with a signing public key (`TMC_UPDATER_PUBKEY`) carries a
 * real updater, and `installable` on the check is how this banner finds out.
 * Then the button installs rather than opening a page — and the signature
 * verified against that compiled-in key is the only thing making that safe,
 * which is why a build without one still shows the link and says so.
 *
 * The two are not alternatives to choose between. A platform the updater does
 * not cover — mobile, or a Linux package manager's own build — keeps the link,
 * and it is the honest floor rather than dead code.
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

import { messageOf } from '~/lib/ipc'
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
    const [installing, setInstalling] = useState(false)
    const [installed, setInstalled] = useState(false)
    const [failed, setFailed] = useState<string | null>(null)

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

    /*
     * Installing does NOT relaunch, deliberately — see `update_install`. So the
     * banner has a third state, and it is the one that matters: the update is
     * on disk and takes effect on the next start. Saying "done" and leaving
     * somebody on the old version is how an updater earns a reputation for not
     * working.
     */
    const install = async () => {
        setInstalling(true)
        setFailed(null)

        try {
            await ipc.updateInstall()
            setInstalled(true)
        } catch (err) {
            setFailed(messageOf(err))
        } finally {
            setInstalling(false)
        }
    }

    if (!found?.latest) return null
    // An installed-but-not-restarted update keeps the banner, because it is the
    // only thing telling somebody to restart.
    if (dismissed === found.latest && !installed) return null

    return (
        <div className="flex items-center gap-3 border-b border-border bg-surface-2 px-3 py-2 text-xs">
            <FiDownload aria-hidden className="size-4 shrink-0 text-accent" />

            <p className="min-w-0 flex-1">
                {installed ? (
                    <>
                        Version {found.latest} is installed. Restart the app to
                        finish.
                    </>
                ) : failed ? (
                    <span className="text-danger">{failed}</span>
                ) : (
                    <>
                        Version {found.latest} is available. You have{' '}
                        {found.current}.
                    </>
                )}
            </p>

            {found.installable ? (
                <button
                    type="button"
                    disabled={installing || installed}
                    onClick={() => void install()}
                    className="shrink-0 rounded-lg bg-accent px-2.5 py-1 font-semibold text-accent-foreground disabled:opacity-60"
                >
                    {installed
                        ? 'Restart to finish'
                        : installing
                          ? 'Installing…'
                          : 'Update now'}
                </button>
            ) : (
                found.download && (
                    <button
                        type="button"
                        onClick={() => void openUrl(found.download!)}
                        className="shrink-0 rounded-lg bg-accent px-2.5 py-1 font-semibold text-accent-foreground"
                    >
                        Get it
                    </button>
                )
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
