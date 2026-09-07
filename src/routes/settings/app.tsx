import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Button } from '@modcommunity/shared'

import { ipc } from '~/lib/ipc/commands'
import { useApiEnv } from '~/lib/api/env'
import { useSettings } from '~/lib/settings/provider'
import { Row, Section, Select, Toggle } from '~/components/form'

/**
 * The speed-limit presets, in bytes per second.
 *
 * A dropdown rather than a number field: the useful answers are all round
 * numbers and a field invites somebody to type `1000` meaning a megabyte and
 * get a kilobyte. `0` is the "no limit" value the settings store uses, so the
 * option and the stored value are the same thing.
 */
const DOWNLOAD_LIMITS = [
    { value: '0', label: 'No limit' },
    { value: String(256 * 1024), label: '256 KB/s' },
    { value: String(512 * 1024), label: '512 KB/s' },
    { value: String(1024 * 1024), label: '1 MB/s' },
    { value: String(2 * 1024 * 1024), label: '2 MB/s' },
    { value: String(5 * 1024 * 1024), label: '5 MB/s' },
    { value: String(10 * 1024 * 1024), label: '10 MB/s' },
    { value: String(25 * 1024 * 1024), label: '25 MB/s' },
    { value: String(50 * 1024 * 1024), label: '50 MB/s' },
]

/**
 * App settings — local to this install, written to `settings.json` by Rust.
 *
 * Nothing here leaves the machine. That is the promise of the App/Account split
 * and it is why the theme, the game directories and the logging switch all live
 * on this side: they describe a machine, and syncing them would push one
 * device's paths onto another.
 */
export default function AppSettingsRoute() {
    const { app, setApp, resetApp } = useSettings()
    const env = useApiEnv()

    // Theme plugins extend the theme picker, so the list has to be live.
    const plugins = useQuery({
        queryKey: ['plugins'],
        queryFn: () => ipc.pluginList(),
        staleTime: 30 * 1000,
    })

    if (!app) return <p className="text-sm text-muted">Loading…</p>

    const themeOptions = [
        { value: 'system', label: 'Match the system' },
        { value: 'light', label: 'Light' },
        { value: 'dark', label: 'Dark' },
        ...(plugins.data ?? [])
            .filter((p) => p.enabled && p.kinds.includes('theme'))
            .map((p) => ({ value: `plugin:${p.id}`, label: p.name })),
    ]

    return (
        <>
            <Section title="Appearance">
                <Row
                    label="Theme"
                    control={
                        <Select
                            label="Theme"
                            value={app.theme}
                            options={themeOptions}
                            onChange={(theme) => void setApp({ theme })}
                        />
                    }
                />
                <Row
                    label="Text size"
                    hint={`${Math.round(app.uiScale * 100)}%`}
                    control={
                        <input
                            type="range"
                            aria-label="Text size"
                            min={0.85}
                            max={1.4}
                            step={0.05}
                            value={app.uiScale}
                            onChange={(e) =>
                                void setApp({ uiScale: Number(e.target.value) })
                            }
                        />
                    }
                />
                <Row
                    label="Compact cards"
                    hint="Fit more on screen."
                    control={
                        <Toggle
                            label="Compact cards"
                            checked={app.compactCards}
                            onChange={(compactCards) =>
                                void setApp({ compactCards })
                            }
                        />
                    }
                />
                <Row
                    label="Show adult content"
                    hint="Applies to every browser in the app."
                    control={
                        <Toggle
                            label="Show adult content"
                            checked={app.showNsfw}
                            onChange={(showNsfw) => void setApp({ showNsfw })}
                        />
                    }
                />
            </Section>

            <Section
                title="Servers"
                hint="Latency is measured by connecting to the server's own port — no third party is involved."
            >
                <Row
                    label="Measure latency"
                    hint="Ping servers as their cards come into view."
                    control={
                        <Toggle
                            label="Measure latency"
                            checked={app.liveLatency}
                            onChange={(liveLatency) => void setApp({ liveLatency })}
                        />
                    }
                />
                <Row
                    label="Refresh every"
                    hint="How often the servers on screen are re-measured."
                    control={
                        <IntervalField
                            valueMs={app.latencyIntervalMs}
                            onChange={(latencyIntervalMs) =>
                                void setApp({ latencyIntervalMs })
                            }
                        />
                    }
                />
                <Row
                    label="Simultaneous pings"
                    hint={String(app.latencyConcurrency)}
                    control={
                        <input
                            type="range"
                            aria-label="Simultaneous pings"
                            min={1}
                            max={32}
                            step={1}
                            value={app.latencyConcurrency}
                            onChange={(e) =>
                                void setApp({
                                    latencyConcurrency: Number(e.target.value),
                                })
                            }
                        />
                    }
                />
            </Section>

            <Section
                title="Downloads"
                hint="Applies to everything the app fetches — a mod's files, a sandbox's staging, a plugin's own downloads."
            >
                <Row
                    label="Speed limit"
                    hint="Keeps the rest of your connection usable while a large modpack downloads."
                    control={
                        <Select
                            label="Download speed limit"
                            value={String(app.downloadLimitBps)}
                            onChange={(next) =>
                                void ipc.downloadSetGlobalLimit(
                                    Number(next) > 0 ? Number(next) : null
                                )
                            }
                            options={DOWNLOAD_LIMITS}
                        />
                    }
                />
                <Row
                    label="At the same time"
                    hint={`${app.downloadConcurrency} file${app.downloadConcurrency === 1 ? '' : 's'} · more is not faster once the link is the bottleneck`}
                    control={
                        <input
                            type="range"
                            aria-label="Simultaneous downloads"
                            min={1}
                            max={8}
                            step={1}
                            value={app.downloadConcurrency}
                            onChange={(e) =>
                                void ipc.downloadSetConcurrency(
                                    Number(e.target.value)
                                )
                            }
                        />
                    }
                />
                <Row
                    label="Keep finished downloads"
                    hint="Leaves them in the list until you clear them."
                    control={
                        <Toggle
                            label="Keep finished downloads"
                            checked={app.downloadKeepHistory}
                            onChange={(downloadKeepHistory) =>
                                void setApp({ downloadKeepHistory })
                            }
                        />
                    }
                />
            </Section>

            <Section title="Plugins">
                <Row
                    label="Confirm every install"
                    hint="Show what a plugin will do each time, not only when you approve it."
                    control={
                        <Toggle
                            label="Confirm every install"
                            checked={app.confirmEveryRun}
                            onChange={(confirmEveryRun) =>
                                void setApp({ confirmEveryRun })
                            }
                        />
                    }
                />

                <Row
                    label="Edit game settings in the app"
                    /*
                     * Honest about what turning it off buys, because a switch
                     * that implies more than it does is worse than no switch.
                     * It removes three commands. It does not sandbox the app,
                     * which writes to game folders whenever it deploys.
                     */
                    hint="Opens the settings files a game declares, from its sandbox. Turning this off removes those commands; it does not change what the app can do when it installs or deploys."
                    control={
                        <Toggle
                            label="Edit game settings in the app"
                            checked={app.allowConfigEditing}
                            onChange={(allowConfigEditing) =>
                                void setApp({ allowConfigEditing })
                            }
                        />
                    }
                />
            </Section>

            <Section
                title="Privacy"
                hint="Security events are always recorded and are not affected by these switches."
            >
                <Row
                    label="Detailed activity log"
                    hint="Record routine actions as well as security ones."
                    control={
                        <Toggle
                            label="Detailed activity log"
                            checked={app.verboseLogging}
                            onChange={(verboseLogging) =>
                                void setApp({ verboseLogging })
                            }
                        />
                    }
                />
                <Row
                    label="Send crash reports"
                    control={
                        <Toggle
                            label="Send crash reports"
                            checked={app.crashReports}
                            onChange={(crashReports) =>
                                void setApp({ crashReports })
                            }
                        />
                    }
                />
            </Section>

            <Section title="Updates">
                <Row
                    label="Check for updates on launch"
                    control={
                        <Toggle
                            label="Check for updates on launch"
                            checked={app.autoUpdateCheck}
                            onChange={(autoUpdateCheck) =>
                                void setApp({ autoUpdateCheck })
                            }
                        />
                    }
                />
            </Section>

            {/*
             * Shown only off production, and read-only wherever it is shown.
             * The title bar carries the same fact on desktop, but a phone has
             * no title bar to carry it — and a dev build on a device is exactly
             * where "which site is this?" is hardest to answer from the screen.
             */}
            {env && !env.isProd && (
                <Section
                    title="Development"
                    hint="This build is not talking to the live site. Set at build time, or through TMC_API_BASE in a debug build's environment — it cannot be changed from in here."
                >
                    <Row
                        label="API"
                        control={
                            <span className="font-mono text-xs text-warning">
                                {env.base}
                            </span>
                        }
                    />
                </Section>
            )}

            {/*
             * Shown on EVERY build, unlike Development above.
             *
             * A version number is the first thing any bug or security report
             * has to carry, and until this existed the only place it appeared
             * was a section that renders on dev builds — so the people most
             * likely to be reporting something were the ones who could not
             * find it. SECURITY.md asks for it by name.
             */}
            {env && (
                <Section title="About">
                    <Row
                        label="Version"
                        control={
                            <span className="font-mono text-xs text-muted">
                                {env.version}
                            </span>
                        }
                    />
                </Section>
            )}

            <div>
                <Button btnType="danger" onClick={() => void resetApp()}>
                    Reset app settings
                </Button>
                <p className="mt-1 text-xs text-muted">
                    Does not sign you out, remove plugins, or touch your account
                    settings.
                </p>
            </div>
        </>
    )
}

/**
 * The bounds, mirroring `LATENCY_INTERVAL_MS_MIN`/`MAX` in Rust.
 *
 * Rust clamps whatever arrives, so these are the UI's manners rather than the
 * enforcement: they keep the input from offering a value that would be silently
 * changed on the way in.
 */
const MIN_MS = 250
const MAX_MS = 300_000

type Unit = 'ms' | 's'

/** Show a whole number of seconds as seconds; anything finer as milliseconds. */
function unitFor(ms: number): Unit {
    return ms % 1000 === 0 ? 's' : 'ms'
}

function inUnit(ms: number, unit: Unit): number {
    return unit === 's' ? ms / 1000 : ms
}

/**
 * A duration, typed in whichever unit suits it.
 *
 * Two controls for one value, because the useful range spans three orders of
 * magnitude: "every 2 seconds" and "every 500 milliseconds" are both ordinary
 * answers here, and a single milliseconds box makes the first of them a
 * four-digit number to read and re-read.
 *
 * The typed text is LOCAL until it is committed, on blur or on Enter. Writing
 * every keystroke through to `settings.json` would make "1500" pass through 1
 * and 15 — two settings the user never chose, each one clamped up to the floor
 * and each one restarting every live query timer in the app.
 */
function IntervalField({
    valueMs,
    onChange,
}: {
    valueMs: number
    onChange: (ms: number) => void
}) {
    const [unit, setUnit] = useState<Unit>(() => unitFor(valueMs))
    const [draft, setDraft] = useState(() =>
        String(inUnit(valueMs, unitFor(valueMs)))
    )

    // Re-sync when the stored value moves underneath us: a reset, or a value
    // the clamp changed on the way in. Deriving it during render rather than in
    // an effect means the clamped number is on screen in the same paint that
    // the rest of the app starts using it.
    const [seen, setSeen] = useState(valueMs)

    if (seen !== valueMs) {
        const next = unitFor(valueMs)

        setSeen(valueMs)
        setUnit(next)
        setDraft(String(inUnit(valueMs, next)))
    }

    const commit = (raw: string, as: Unit) => {
        const typed = Number(raw.trim())

        // Not a number, or not a duration. Put the stored value back rather
        // than storing something the user did not mean.
        if (!Number.isFinite(typed) || typed <= 0) {
            setDraft(String(inUnit(valueMs, as)))

            return
        }

        const ms = Math.min(
            MAX_MS,
            Math.max(MIN_MS, Math.round(as === 's' ? typed * 1000 : typed))
        )

        if (ms === valueMs) setDraft(String(inUnit(ms, as)))
        else onChange(ms)
    }

    return (
        <div className="flex items-center gap-1.5">
            <input
                type="number"
                inputMode="numeric"
                aria-label="Refresh interval"
                title={`Between ${MIN_MS}ms and ${MAX_MS / 1000} seconds.`}
                min={inUnit(MIN_MS, unit)}
                max={inUnit(MAX_MS, unit)}
                step={unit === 's' ? 0.5 : 50}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onBlur={(e) => commit(e.target.value, unit)}
                onKeyDown={(e) => {
                    if (e.key === 'Enter') commit(e.currentTarget.value, unit)
                }}
                className="w-24 rounded-lg border border-border bg-background px-2 py-1.5 text-right text-sm tabular-nums"
            />

            <Select<Unit>
                label="Refresh interval unit"
                value={unit}
                options={[
                    { value: 's', label: 'seconds' },
                    { value: 'ms', label: 'ms' },
                ]}
                onChange={(next) => {
                    // The unit converts the value rather than reinterpreting
                    // it: switching from "1 second" to milliseconds means 1000,
                    // not a 1ms interval nobody asked for.
                    setUnit(next)
                    setDraft(String(inUnit(valueMs, next)))
                }}
            />
        </div>
    )
}
