import { NavLink, Outlet } from 'react-router-dom'

/**
 * Settings, split the way the data is split.
 *
 * "App" and "Account" are separate top-level panes rather than sections of one
 * list, because they answer different questions and fail differently: an app
 * setting is written to disk and always succeeds; an account setting is a
 * request to the website and can be refused, be stale, or be unavailable when
 * signed out. Mixing them into one screen makes that invisible.
 */
const PANES = [
    { to: '/settings', end: true, label: 'App' },
    { to: '/settings/account', label: 'Account' },
    { to: '/settings/games', label: 'Games' },
    { to: '/settings/plugins', label: 'Plugins' },
    { to: '/settings/logging', label: 'Logging' },
]

export default function SettingsLayout() {
    return (
        <div className="flex h-full flex-col">
            <nav className="flex shrink-0 gap-1 overflow-x-auto border-b border-border bg-surface px-3 py-2">
                {PANES.map((pane) => (
                    <NavLink
                        key={pane.to}
                        to={pane.to}
                        end={pane.end}
                        className={({ isActive }) =>
                            `shrink-0 rounded-full px-3 py-1.5 text-xs transition-colors ${
                                isActive
                                    ? 'bg-accent text-accent-foreground'
                                    : 'text-muted hover:bg-surface-hover'
                            }`
                        }
                    >
                        {pane.label}
                    </NavLink>
                ))}
            </nav>

            <div className="min-h-0 flex-1 overflow-y-auto p-4">
                <div className="mx-auto flex max-w-2xl flex-col gap-6">
                    <Outlet />
                </div>
            </div>
        </div>
    )
}
