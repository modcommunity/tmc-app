import { useEffect, useState } from 'react'
import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { FiChevronRight, FiUser } from 'react-icons/fi'

import { useAuth } from '~/lib/auth/provider'
import { useDownloads } from '~/lib/downloads/provider'
import { useDeepLink } from '~/lib/hooks/use-deep-link'
import { useIsCompact } from '~/lib/hooks/use-breakpoint'
import { useIsDesktop } from '~/lib/hooks/use-platform'
import { openUrl } from '@tauri-apps/plugin-opener'
import { GsLogo } from './gs-logo'
import Titlebar, { WindowResizeEdges } from './titlebar'
import UpdateBanner from './update-banner'
import OfflineBanner from './offline-banner'
import { COMPACT, RAIL, isActive, isWithin, isOwn, type NavNode } from '~/lib/nav'

/**
 * The app's chrome.
 *
 * This is where the app deliberately stops looking like the website. The site
 * has a marketing header, a mega-menu and a footer full of links; an app has a
 * persistent primary navigation and nothing else, because the user is not
 * arriving from a search engine and does not need to be told what GameSpice is on
 * every screen.
 *
 * One component covers all five platforms rather than two. The tree it draws
 * is in lib/nav; the bottom bar cannot nest, so it shows the top level only and
 * the Discover and Library landing pages link to every child:
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

/** localStorage key for which collapsible groups the user left open. */
const OPEN_KEY = 'tmc.nav.open'

function readOpen(): Record<string, boolean> {
    try {
        const raw = localStorage.getItem(OPEN_KEY)
        const parsed: unknown = raw ? JSON.parse(raw) : {}

        return parsed && typeof parsed === 'object'
            ? (parsed as Record<string, boolean>)
            : {}
    } catch {
        // Private mode, a blocked store or a corrupt value: the rail still
        // works, it just forgets.
        return {}
    }
}

function writeOpen(open: Record<string, boolean>) {
    try {
        localStorage.setItem(OPEN_KEY, JSON.stringify(open))
    } catch {
        // See readOpen.
    }
}

function collapsibleIds(nodes: NavNode[]): string[] {
    return nodes.flatMap((node) => [
        ...(node.collapsible ? [node.id] : []),
        ...collapsibleIds(node.items ?? []),
    ])
}

const rowBase =
    'no-drag flex min-w-0 items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors'
const rowOn = 'gs-rail-on'
// A parent of the active child: visible without competing with the child.
const rowSoft = 'bg-accent/15 text-foreground'
const rowOff = 'text-muted hover:bg-surface-hover hover:text-foreground focus-visible:outline-2 focus-visible:outline-accent'

function DownloadsBadge() {
    const downloads = useDownloads()

    if (downloads.active === 0) return null

    return (
        <span className="ml-auto rounded-full bg-accent px-1.5 py-0.5 text-[0.6rem] font-medium text-accent-foreground">
            {downloads.active}
        </span>
    )
}

function RailItem({
    node,
    pathname,
    open,
    toggle,
}: {
    node: NavNode
    pathname: string
    open: Record<string, boolean>
    toggle: (id: string) => void
}) {
    const items = node.items ?? []
    const own = isOwn(node, pathname)
    const within = isWithin(node, pathname)
    const tone = own ? rowOn : within ? rowSoft : rowOff
    const expanded = !node.collapsible || open[node.id] === true
    const panelId = `nav-${node.id}`

    const label = (
        <>
            <node.icon className="size-4 shrink-0" />
            <span className="truncate">{node.label}</span>
            {node.badge === 'downloads' && <DownloadsBadge />}
        </>
    )

    return (
        <div>
            <div className="flex items-center gap-0.5">
                {node.virtual ? (
                    // No screen of its own: the whole header is the toggle.
                    <button
                        type="button"
                        onClick={() => toggle(node.id)}
                        aria-expanded={expanded}
                        aria-controls={panelId}
                        className={`${rowBase} ${tone} flex-1 text-left`}
                    >
                        {label}
                        <FiChevronRight
                            className={`ml-auto size-3.5 shrink-0 transition-transform duration-200 ${expanded ? 'rotate-90' : ''}`}
                        />
                    </button>
                ) : (
                    <>
                        <NavLink
                            to={node.to}
                            className={`${rowBase} ${tone} flex-1`}
                        >
                            {label}
                        </NavLink>
                        {node.collapsible && (
                            <button
                                type="button"
                                onClick={() => toggle(node.id)}
                                aria-expanded={expanded}
                                aria-controls={panelId}
                                aria-label={`${expanded ? 'Collapse' : 'Expand'} ${node.label}`}
                                className="no-drag flex size-8 shrink-0 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
                            >
                                <FiChevronRight
                                    className={`size-3.5 transition-transform duration-200 ${expanded ? 'rotate-90' : ''}`}
                                />
                            </button>
                        )}
                    </>
                )}
            </div>

            {items.length > 0 && (
                /*
                 * grid-template-rows 0fr to 1fr is the one way to animate to an
                 * unknown height in CSS; a max-height guess clips or lags.
                 * `inert` takes the collapsed links out of the tab order, which
                 * overflow-hidden alone does not.
                 */
                <div
                    id={panelId}
                    inert={!expanded}
                    className={`grid transition-[grid-template-rows,opacity] duration-200 ease-out ${expanded ? 'grid-rows-[1fr] opacity-100' : 'grid-rows-[0fr] opacity-0'}`}
                >
                    <div className="overflow-hidden">
                        <div className="ml-5 mt-0.5 flex flex-col gap-0.5 border-l border-border pl-2">
                            {items.map((child) => (
                                <RailItem
                                    key={child.id}
                                    node={child}
                                    pathname={pathname}
                                    open={open}
                                    toggle={toggle}
                                />
                            ))}
                        </div>
                    </div>
                </div>
            )}
        </div>
    )
}

function Rail({ pathname }: { pathname: string }) {
    const [open, setOpen] = useState<Record<string, boolean>>(readOpen)

    /*
     * Opening a group when the route lands on or inside it is stored like a click, so
     * collapsing it afterwards sticks. Deriving "open" from the route instead
     * would make the chevron do nothing while you are inside the group.
     */
    useEffect(() => {
        const wanted = collapsibleIds(RAIL).filter((id) => {
            const find = (nodes: NavNode[]): NavNode | undefined => {
                for (const n of nodes) {
                    if (n.id === id) return n
                    const hit = find(n.items ?? [])
                    if (hit) return hit
                }
            }
            const node = find(RAIL)

            return node && isActive(node, pathname)
        })

        if (wanted.every((id) => open[id])) return

        // eslint-disable-next-line react-hooks/set-state-in-effect -- reacting to the route, which is external state
        setOpen((prev) => {
            const next = { ...prev }
            for (const id of wanted) next[id] = true
            writeOpen(next)

            return next
        })
        // `open` is read only to skip a no-op write.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [pathname])

    const toggle = (id: string) =>
        setOpen((prev) => {
            const next = { ...prev, [id]: !prev[id] }
            writeOpen(next)

            return next
        })

    return (
        <div className="flex flex-1 flex-col gap-1 overflow-y-auto px-2 text-sm">
            {RAIL.map((node) => (
                <RailItem
                    key={node.id}
                    node={node}
                    pathname={pathname}
                    open={open}
                    toggle={toggle}
                />
            ))}
        </div>
    )
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
        <div className="flex h-screen w-screen flex-col overflow-hidden text-foreground">
            {desktop && (
                <>
                    <Titlebar title="GameSpice App" />
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
                            <GsLogo size={34} id="rail" />
                            <span className="gs-wordmark text-lg leading-none">
                                Game<span className="gs-flame-text">Spice</span>
                            </span>
                        </div>

                        <Rail pathname={pathname} />

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
                    <OfflineBanner />

                    <div
                        className="min-h-0 flex-1 overflow-y-auto"
                        style={{
                            paddingTop: compact ? 'var(--safe-top)' : undefined,
                        }}
                    >
                        <div key={pathname.split('/')[1]} className="gs-fade min-h-full">
                            <Outlet />
                        </div>
                    </div>

                    {/*
                     * A row of its own below the scroll area, so it never
                     * covers content, and above the bottom bar on a phone so
                     * the bar never covers it.
                     */}
                    <div className="flex shrink-0 justify-end px-3 py-0.5">
                        <button
                            type="button"
                            onClick={() => void openUrl('https://modcommunity.net')}
                            className="no-drag text-[11px] leading-4 text-muted/70 transition-colors hover:text-foreground"
                        >
                            Powered by TMC
                        </button>
                    </div>

                    {compact && (
                        <nav
                            className="flex shrink-0 items-stretch border-t border-border bg-surface"
                            style={{ paddingBottom: 'var(--safe-bottom)' }}
                        >
                            {COMPACT.map((tab) => (
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
                                        {tab.id === 'library' &&
                                            downloads.active > 0 && (
                                                <span className="absolute -right-2 -top-1 min-w-3.5 rounded-full bg-accent px-1 text-[0.55rem] font-medium leading-tight text-accent-foreground">
                                                    {downloads.active}
                                                </span>
                                            )}
                                    </span>
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
