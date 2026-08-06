import { useQuery } from '@tanstack/react-query'
import { Button } from '@modcommunity/shared'

import { ipc } from '~/lib/ipc/commands'
import { useSettings } from '~/lib/settings/provider'
import { Row, Section, Select, Toggle } from '~/components/form'

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
                            onChange={(compactCards) => void setApp({ compactCards })}
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
                            onChange={(crashReports) => void setApp({ crashReports })}
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
