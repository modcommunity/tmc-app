import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { Logo } from '@modcommunity/shared'
import {
    FiBox,
    FiPackage,
    FiServer,
    FiGrid,
    FiSettings,
    FiUser,
    FiDownloadCloud,
} from 'react-icons/fi'
import type { IconType } from 'react-icons'

import { useAuth } from '~/lib/auth/provider'
import { useIsCompact } from '~/lib/hooks/use-breakpoint'
import { useIsDesktop } from '~/lib/hooks/use-platform'
import Titlebar, { WindowResizeEdges } from './titlebar'

/**
 * The app's chrome.
 *
 * This is where the app deliberately stops looking like the website. The site
 * has a marketing header, a mega-menu and a footer full of links; an app has a
 * persistent primary navigation and nothing else, because the user is not
 * arriving from a search engine and does not need to be told what TMC is on
 * every screen.
 *
 * One component covers all five platforms rather than two:
 *
 *   * **≥ 768px** — a rail on the left. Wide windows and tablets in landscape.
 *   * **< 768px** — a bottom tab bar. Thumb-reachable, which a top bar is not
 *     on a phone, and it is what every native app on both mobile platforms
 *     does.
 *
 * The breakpoint is on the WINDOW, not on the OS: a desktop window dragged
 * narrow gets the phone layout, which is the correct answer and also the only
 * way to test the mobile shell without a device.
 */

type Tab = {
    to: string
    label: string
    icon: IconType
    /** Also highlight for these path prefixes. */
    match?: string[]
}

const TABS: Tab[] = [
    { to: '/browse/mod', label: 'Mods', icon: FiPackage, match: ['/view/mod'] },
    { to: '/browse/asset', label: 'Assets', icon: FiBox, match: ['/view/asset'] },
    {
        to: '/browse/server',
        label: 'Servers',
        icon: FiServer,
        match: ['/view/server', '/view/serverMap', '/browse/serverMap'],
    },
    {
        to: '/browse/community',
        label: 'Discover',
        icon: FiGrid,
        match: [
            '/browse/article',
            '/browse/collection',
            '/browse/user',
            '/view/article',
            '/view/community',
            '/view/collection',
            '/view/user',
        ],
    },
    {
        // The device's own half of the app: what is subscribed, what is
        // installed, and the sandboxes it is installed into. Sits before
        // Settings because it is a place a user goes to DO something.
        to: '/library',
        label: 'Library',
        icon: FiDownloadCloud,
        match: ['/installs'],
    },
    { to: '/settings', label: 'Settings', icon: FiSettings },
]

function isActive(tab: Tab, pathname: string): boolean {
    if (pathname === tab.to || pathname.startsWith(`${tab.to}/`)) return true

    return (tab.match ?? []).some((prefix) => pathname.startsWith(prefix))
}

export default function Shell() {
    const compact = useIsCompact()
    const desktop = useIsDesktop()
    const { pathname } = useLocation()
    const { user, status } = useAuth()

    /*
     * On desktop the app draws its own frame — see components/titlebar. The
     * bar sits ABOVE the rail and the content both, so it spans the window the
     * way a real titlebar does rather than starting after the sidebar.
     */
    return (
        <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground">
            {desktop && (
                <>
                    <Titlebar title="The Modding Community" />
                    <WindowResizeEdges />
                </>
            )}

            <div
                className="flex min-h-0 flex-1"
                style={{
                    paddingLeft: 'var(--safe-left)',
                    paddingRight: 'var(--safe-right)',
                }}
            >
                {!compact && (
                    <nav className="flex w-56 shrink-0 flex-col border-r border-border bg-surface">
                        <div
                            className="flex items-center gap-2 px-4 py-4"
                            style={{
                                paddingTop: desktop
                                    ? '1rem'
                                    : 'calc(1rem + var(--safe-top))',
                            }}
                        >
                            <Logo />
                        </div>

                        <div className="flex flex-1 flex-col gap-1 overflow-y-auto px-2">
                            {TABS.map((tab) => (
                                <NavLink
                                    key={tab.to}
                                    to={tab.to}
                                    className={`no-drag flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors ${
                                        isActive(tab, pathname)
                                            ? 'bg-accent text-accent-foreground'
                                            : 'text-muted hover:bg-surface-hover hover:text-foreground'
                                    }`}
                                >
                                    <tab.icon className="size-4 shrink-0" />
                                    {tab.label}
                                </NavLink>
                            ))}
                        </div>

                        <NavLink
                            to="/account"
                            className="no-drag m-2 flex items-center gap-3 rounded-lg px-3 py-2 text-sm text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
                            style={{
                                marginBottom: 'calc(0.5rem + var(--safe-bottom))',
                            }}
                        >
                            {user?.avatar ? (
                                <img
                                    src={user.avatar}
                                    alt=""
                                    className="size-6 shrink-0 rounded-full object-cover"
                                />
                            ) : (
                                <FiUser className="size-4 shrink-0" />
                            )}
                            <span className="truncate">
                                {status === 'signedIn'
                                    ? (user?.username ?? user?.name ?? 'Account')
                                    : 'Sign in'}
                            </span>
                        </NavLink>
                    </nav>
                )}

                <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
                    <div
                        className="min-h-0 flex-1 overflow-y-auto"
                        style={{
                            paddingTop: compact ? 'var(--safe-top)' : undefined,
                        }}
                    >
                        <Outlet />
                    </div>

                    {compact && (
                        <nav
                            className="flex shrink-0 items-stretch border-t border-border bg-surface"
                            style={{ paddingBottom: 'var(--safe-bottom)' }}
                        >
                            {TABS.map((tab) => (
                                <NavLink
                                    key={tab.to}
                                    to={tab.to}
                                    className={`flex flex-1 flex-col items-center gap-1 py-2 text-[0.65rem] transition-colors ${
                                        isActive(tab, pathname)
                                            ? 'text-accent'
                                            : 'text-muted'
                                    }`}
                                >
                                    <tab.icon className="size-5" />
                                    {tab.label}
                                </NavLink>
                            ))}
                        </nav>
                    )}
                </main>
            </div>
        </div>
    )
}
