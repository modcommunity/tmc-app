import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { Logo } from '@modcommunity/shared'
import {
    FiBox,
    FiPackage,
    FiServer,
    FiCompass,
    FiGrid,
    FiSettings,
    FiUser,
    FiDownload,
    FiDownloadCloud,
    FiLayers,
    FiTerminal,
} from 'react-icons/fi'
import type { IconType } from 'react-icons'

import { useAuth } from '~/lib/auth/provider'
import { useDownloads } from '~/lib/downloads/provider'
import { useDeepLink } from '~/lib/hooks/use-deep-link'
import { useIsCompact } from '~/lib/hooks/use-breakpoint'
import { useIsDesktop } from '~/lib/hooks/use-platform'
import Titlebar, { WindowResizeEdges } from './titlebar'
import UpdateBanner from './update-banner'

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
    /**
     * Show the count of active downloads on this tab.
     *
     * Only one tab carries it, and it is the reason downloads have a tab at
     * all: a queue nobody can see the state of from another screen is one
     * people check by opening it, which is the thing a badge exists to save.
     */
    badge?: 'downloads'
    /**
     * Show this in the bottom tab bar as well as the rail.
     *
     * Default true. Turned off for the two screens that are deep work rather
     * than navigation — a phone-width bar with nine items in it is a row of
     * unreadable four-pixel labels, and the fix is fewer items rather than
     * smaller text. Both are still reachable, from the Library screen.
     */
    compact?: boolean
}

const TABS: Tab[] = [
    {
        // The front door, and the app's own answer to a question the website
        // cannot ask: what is there, and which of it can I start right now.
        // First because a flat catalogue across every game is what this app
        // opens on — the site has to make you choose a game before it can show
        // you anything.
        to: '/apps',
        label: 'Apps',
        icon: FiGrid,
    },
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
        icon: FiCompass,
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
    {
        // The mod manager proper. Its own tab rather than a tab inside Library
        // because it is where somebody SPENDS time — reordering, deploying,
        // checking what conflicted — while the library is a list they glance
        // at.
        to: '/sandboxes',
        label: 'Sandboxes',
        icon: FiLayers,
        compact: false,
    },
    {
        to: '/downloads',
        label: 'Downloads',
        icon: FiDownload,
        badge: 'downloads',
    },
    { to: '/rcon', label: 'Console', icon: FiTerminal, compact: false },
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
    const downloads = useDownloads()

    /*
     * Inside the router, because a deep link's only power is to navigate — and
     * `useNavigate` needs a router above it. Rust has already refused anything
     * that is not one of a handful of bounded shapes.
     */
    useDeepLink()

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
                                    <span className="truncate">{tab.label}</span>
                                    {tab.badge === 'downloads' &&
                                        downloads.active > 0 && (
                                            <span className="ml-auto rounded-full bg-accent px-1.5 py-0.5 text-[0.6rem] font-medium text-accent-foreground">
                                                {downloads.active}
                                            </span>
                                        )}
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
                    {/*
                     * Above the scrolling area rather than inside it, so it is
                     * not something a user scrolls past and never sees on a
                     * long browse page. It renders nothing at all unless the
                     * check ran, found a newer version, and the user has not
                     * dismissed that one.
                     */}
                    <UpdateBanner />

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
                            {TABS.filter((tab) => tab.compact !== false).map(
                                (tab) => (
                                    <NavLink
                                        key={tab.to}
                                        to={tab.to}
                                        className={`flex flex-1 flex-col items-center gap-1 py-2 text-[0.65rem] transition-colors ${
                                            isActive(tab, pathname)
                                                ? 'text-accent'
                                                : 'text-muted'
                                        }`}
                                    >
                                        <span className="relative">
                                            <tab.icon className="size-5" />
                                            {tab.badge === 'downloads' &&
                                                downloads.active > 0 && (
                                                    <span className="absolute -right-2 -top-1 min-w-3.5 rounded-full bg-accent px-1 text-[0.55rem] font-medium leading-tight text-accent-foreground">
                                                        {downloads.active}
                                                    </span>
                                                )}
                                        </span>
                                        {tab.label}
                                    </NavLink>
                                )
                            )}
                        </nav>
                    )}
                </main>
            </div>
        </div>
    )
}
