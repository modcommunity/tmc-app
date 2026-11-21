import { useEffect, useState } from 'react'
import { z } from 'zod'

import { call } from '~/lib/ipc'

import { GsLogo } from './gs-logo'

/**
 * The menu shown when the system tray icon is right-clicked.
 *
 * It lives in its own small borderless window (see `open_tray_menu` in
 * `src-tauri/src/notifier.rs`), which loads this same bundle with
 * `window.__GS_TRAY__` set. A row only names a fixed action; Rust decides what
 * that does. The window closes itself when it loses focus.
 */
export interface TrayData {
    version: string
}

declare global {
    interface Window {
        __GS_TRAY__?: TrayData
    }
}

const Nothing = z.null().or(z.undefined())

interface Row {
    action: string
    label: string
    glyph: string
    danger?: boolean
}

const ROWS: Row[] = [
    { action: 'open', label: 'Open GameSpice', glyph: '◧' },
    { action: 'play', label: 'Play', glyph: '▶' },
    { action: 'downloads', label: 'Downloads', glyph: '⇩' },
    { action: 'settings', label: 'Settings', glyph: '⚙' },
    { action: 'quit', label: 'Quit GameSpice', glyph: '⏻', danger: true },
]

export default function TrayMenu({ data }: { data: TrayData }) {
    const [active, setActive] = useState(0)

    useEffect(() => {
        document.documentElement.style.background = '#140c09'
        document.body.style.margin = '0'
        document.body.style.overflow = 'hidden'
    }, [])

    useEffect(() => {
        const choose = (action: string) =>
            void call('tray_action', Nothing, { action })

        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') choose('')
            else if (e.key === 'ArrowDown') setActive((i) => (i + 1) % ROWS.length)
            else if (e.key === 'ArrowUp')
                setActive((i) => (i + ROWS.length - 1) % ROWS.length)
            else if (e.key === 'Enter') choose(ROWS[active]?.action ?? '')
        }

        window.addEventListener('keydown', onKey)

        return () => window.removeEventListener('keydown', onKey)
    }, [active])

    return (
        <div
            role="menu"
            style={{
                position: 'fixed',
                inset: 0,
                display: 'flex',
                flexDirection: 'column',
                boxSizing: 'border-box',
                padding: 8,
                background: 'linear-gradient(160deg, #241510 0%, #140c09 75%)',
                border: '1px solid #e4572e44',
                borderRadius: 12,
                color: '#fff6e6',
                font: '13px/1.3 system-ui, -apple-system, "Segoe UI", sans-serif',
                userSelect: 'none',
            }}
        >
            <div
                style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: '6px 8px 10px',
                    borderBottom: '1px solid #ffffff12',
                    marginBottom: 6,
                }}
            >
                <GsLogo size={30} id="tray" />
                <div style={{ minWidth: 0 }}>
                    <div style={{ fontWeight: 700, fontSize: 14 }}>GameSpice</div>
                    <div style={{ color: '#d9b99f', fontSize: 11 }}>v{data.version}</div>
                </div>
            </div>
            {ROWS.map((row, i) => {
                const on = i === active

                return (
                    <button
                        key={row.action}
                        type="button"
                        role="menuitem"
                        onMouseEnter={() => setActive(i)}
                        onClick={() => void call('tray_action', Nothing, { action: row.action })}
                        style={{
                            display: 'flex',
                            alignItems: 'center',
                            gap: 10,
                            width: '100%',
                            padding: '8px 10px',
                            marginTop: row.danger ? 6 : 0,
                            border: 0,
                            borderRadius: 8,
                            textAlign: 'left',
                            cursor: 'pointer',
                            font: 'inherit',
                            color: row.danger ? '#ff8a7a' : '#fff6e6',
                            background: on
                                ? row.danger
                                    ? '#ff5a4a22'
                                    : 'linear-gradient(90deg, #ffb02e33, #f2542d33)'
                                : 'transparent',
                            boxShadow: on
                                ? `inset 0 0 0 1px ${row.danger ? '#ff5a4a55' : '#f2542d66'}`
                                : 'none',
                        }}
                    >
                        <span
                            aria-hidden
                            style={{
                                width: 18,
                                textAlign: 'center',
                                color: row.danger ? '#ff8a7a' : '#ffb02e',
                            }}
                        >
                            {row.glyph}
                        </span>
                        {row.label}
                    </button>
                )
            })}
        </div>
    )
}
