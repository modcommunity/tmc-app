import { useCallback, useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { FiMaximize2, FiMinimize2, FiMinus, FiX } from 'react-icons/fi'

import { useApiEnv } from '~/lib/api/env'
import { usePlatform } from '~/lib/hooks/use-platform'

/**
 * The app's own window frame.
 *
 * WHY THE APP DRAWS THIS AT ALL
 * -----------------------------
 * Tauri's Linux backend is WebKitGTK, so a decorated window gets a GTK
 * titlebar — which is themed by whatever the user's desktop happens to be set
 * to, and matches neither the app nor the other four platforms it ships on.
 * There is no Qt backend to switch to; the way to stop a Tauri app looking like
 * a GTK app is to stop letting GTK draw any of it.
 *
 * So `decorations` is off in `tauri.conf.json` and everything a title bar does
 * is reimplemented here: dragging, double-click to maximise, the buttons, and
 * the resize edges the window manager is no longer providing.
 *
 * DRAGGING IS AN ATTRIBUTE, NOT A CSS PROPERTY
 * --------------------------------------------
 * `-webkit-app-region: drag` is a Chromium extension. It works in the WebView2
 * shell on Windows and does nothing at all in WebKitGTK or WKWebView — which is
 * both platforms this frame most needs to work on. `data-tauri-drag-region` is
 * Tauri's own, handled in the shell, and works everywhere.
 */

/** Width of the invisible strips that stand in for the WM's resize border. */
const GRIP = 4

type Edge = {
    dir:
        | 'North'
        | 'South'
        | 'East'
        | 'West'
        | 'NorthEast'
        | 'NorthWest'
        | 'SouthEast'
        | 'SouthWest'
    className: string
    cursor: string
    width?: number
    height?: number
}

/*
 * Eight strips around the window: four edges and four corners, corners last so
 * they stack above the edges they overlap. Without these a decorationless
 * window cannot be resized at all on Linux — the WM was the only thing offering
 * a grab area, and turning decorations off took it away.
 */
const CORNER = GRIP * 3

const EDGES: Edge[] = [
    {
        dir: 'North',
        className: 'left-0 right-0 top-0',
        cursor: 'ns-resize',
        height: GRIP,
    },
    {
        dir: 'South',
        className: 'bottom-0 left-0 right-0',
        cursor: 'ns-resize',
        height: GRIP,
    },
    {
        dir: 'West',
        className: 'bottom-0 left-0 top-0',
        cursor: 'ew-resize',
        width: GRIP,
    },
    {
        dir: 'East',
        className: 'bottom-0 right-0 top-0',
        cursor: 'ew-resize',
        width: GRIP,
    },

    // Corners last, so they paint over the edges they overlap and a diagonal
    // grab does not turn into a one-axis one.
    {
        dir: 'NorthWest',
        className: 'left-0 top-0',
        cursor: 'nwse-resize',
        width: CORNER,
        height: CORNER,
    },
    {
        dir: 'NorthEast',
        className: 'right-0 top-0',
        cursor: 'nesw-resize',
        width: CORNER,
        height: CORNER,
    },
    {
        dir: 'SouthWest',
        className: 'bottom-0 left-0',
        cursor: 'nesw-resize',
        width: CORNER,
        height: CORNER,
    },
    {
        dir: 'SouthEast',
        className: 'bottom-0 right-0',
        cursor: 'nwse-resize',
        width: CORNER,
        height: CORNER,
    },
]

export function WindowResizeEdges() {
    const start = useCallback((dir: Edge['dir']) => {
        void getCurrentWindow().startResizeDragging(dir)
    }, [])

    return (
        <>
            {EDGES.map((edge) => (
                <div
                    key={edge.dir}
                    onMouseDown={(e) => {
                        // Left button only: a right-click here would otherwise
                        // begin a resize the user cannot cancel.
                        if (e.button === 0) start(edge.dir)
                    }}
                    className={`fixed z-50 ${edge.className}`}
                    style={{
                        cursor: edge.cursor,
                        width: edge.width,
                        height: edge.height,
                    }}
                />
            ))}
        </>
    )
}

function ControlButton({
    onClick,
    label,
    danger = false,
    children,
}: {
    onClick: () => void
    label: string
    danger?: boolean
    children: React.ReactNode
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            aria-label={label}
            title={label}
            className={`flex h-full w-11 items-center justify-center text-muted transition-colors ${
                danger
                    ? 'hover:bg-danger hover:text-danger-foreground'
                    : 'hover:bg-surface-hover hover:text-foreground'
            }`}
        >
            {children}
        </button>
    )
}

/**
 * The host this build is talking to, shown only when it is not the real one.
 *
 * Nothing else on screen distinguishes a dev instance from production — the
 * layout, the branding and the data all look the same — so without this a
 * screenshot of a staging database is indistinguishable from a report about
 * live data. Rendered in the title bar rather than in Settings because the
 * point is to be visible without being looked for.
 */
function ApiBaseBadge() {
    const env = useApiEnv()

    if (!env || env.isProd) return null

    // The scheme is noise in an 8px-tall bar; the host and port are the fact.
    const host = env.base.replace(/^https?:\/\//, '')

    return (
        <span
            data-tauri-drag-region
            title={`API: ${env.base}`}
            className="ml-2 shrink-0 rounded bg-warning/15 px-1.5 py-0.5 font-mono text-[10px] leading-none text-warning uppercase"
        >
            {host}
        </span>
    )
}

export default function Titlebar({ title }: { title: string }) {
    const platform = usePlatform()
    const [maximized, setMaximized] = useState(false)

    /*
     * Tracked rather than assumed: the window can be maximised by the WM — a
     * keyboard shortcut, a snap gesture, a double-click on the drag region —
     * and the button's icon has to follow it rather than only its own clicks.
     */
    useEffect(() => {
        const win = getCurrentWindow()
        let live = true

        const sync = () => {
            void win.isMaximized().then((value) => {
                if (live) setMaximized(value)
            })
        }

        sync()

        const unlisten = win.onResized(sync)

        return () => {
            live = false
            void unlisten.then((fn) => fn())
        }
    }, [])

    const win = getCurrentWindow()

    const controls = (
        <div className="no-drag flex h-full shrink-0 items-stretch">
            <ControlButton onClick={() => void win.minimize()} label="Minimise">
                <FiMinus className="size-4" />
            </ControlButton>
            <ControlButton
                onClick={() => void win.toggleMaximize()}
                label={maximized ? 'Restore' : 'Maximise'}
            >
                {maximized ? (
                    <FiMinimize2 className="size-3.5" />
                ) : (
                    <FiMaximize2 className="size-3.5" />
                )}
            </ControlButton>
            <ControlButton onClick={() => void win.close()} label="Close" danger>
                <FiX className="size-4" />
            </ControlButton>
        </div>
    )

    // macOS puts its window controls on the left, and a user of that platform
    // reads buttons on the right as a foreign app. The rest of the bar is
    // identical, so this is the only concession the layout makes to the OS.
    const macos = platform === 'macos'

    return (
        <header
            data-tauri-drag-region
            onDoubleClick={() => void win.toggleMaximize()}
            className="flex h-8 shrink-0 items-stretch justify-between border-b border-app-chrome-border bg-app-chrome select-none"
        >
            {macos && controls}

            <div
                data-tauri-drag-region
                className="flex min-w-0 flex-1 items-center px-3 text-xs text-muted"
                style={macos ? { justifyContent: 'center' } : undefined}
            >
                <span data-tauri-drag-region className="truncate">
                    {title}
                </span>

                <ApiBaseBadge />
            </div>

            {!macos && controls}
        </header>
    )
}
