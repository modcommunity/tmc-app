import { useEffect, useRef, useState } from 'react'
import { z } from 'zod'

import { call } from '~/lib/ipc'

/**
 * The notification card shown in the corner of the screen.
 *
 * It lives in its own small borderless window (see `src-tauri/src/notifier.rs`),
 * which loads this same bundle with `window.__GS_TOAST__` set. Everything here
 * is rendered as TEXT — the strings came from the network — and the card
 * cannot say where a click goes: Rust looks that up from what it stored.
 */
export interface ToastData {
    notice: {
        id: string
        title: string
        body: string
        kind: string
        actor?: { name?: string | null; avatarUrl?: string | null } | null
    }
    style: { accent: string; glyph: string; label: string }
}

declare global {
    interface Window {
        __GS_TOAST__?: ToastData
    }
}

const SHOW_MS = 8000
const Nothing = z.null().or(z.undefined())

/** Only a plain #rrggbb reaches a style attribute; anything else is the brand colour. */
function safeColour(v: string): string {
    return /^#[0-9a-fA-F]{6}$/.test(v) ? v : '#e4572e'
}

export default function NotificationToast({ data }: { data: ToastData }) {
    const { notice, style } = data
    const accent = safeColour(style.accent)
    const [paused, setPaused] = useState(false)
    const left = useRef(SHOW_MS)

    useEffect(() => {
        document.documentElement.style.background = '#1a0f0c'
        document.body.style.margin = '0'
        document.body.style.overflow = 'hidden'
    }, [])

    useEffect(() => {
        if (paused) return

        const started = Date.now()
        const timer = window.setTimeout(
            () => void call('toast_dismiss', Nothing),
            left.current
        )

        return () => {
            window.clearTimeout(timer)
            left.current = Math.max(1200, left.current - (Date.now() - started))
        }
    }, [paused])

    return (
        <div
            role="alert"
            onMouseEnter={() => setPaused(true)}
            onMouseLeave={() => setPaused(false)}
            onClick={() => void call('toast_activate', Nothing)}
            style={{
                position: 'fixed',
                inset: 0,
                display: 'flex',
                overflow: 'hidden',
                cursor: 'pointer',
                background:
                    'linear-gradient(135deg, #241510 0%, #1a0f0c 70%)',
                color: '#fff6e6',
                font: '13px/1.35 system-ui, -apple-system, "Segoe UI", sans-serif',
                border: `1px solid ${accent}55`,
                borderRadius: 12,
                boxSizing: 'border-box',
            }}
        >
            <div style={{ width: 5, background: accent, flex: 'none' }} />
            <div
                aria-hidden
                style={{
                    flex: 'none',
                    width: 44,
                    height: 44,
                    margin: '14px 0 0 12px',
                    borderRadius: 12,
                    display: 'grid',
                    placeItems: 'center',
                    fontSize: 22,
                    background: `${accent}2e`,
                    boxShadow: `inset 0 0 0 1px ${accent}66`,
                }}
            >
                {style.glyph}
            </div>
            <div style={{ flex: 1, minWidth: 0, padding: '12px 30px 0 12px' }}>
                <div
                    style={{
                        color: accent,
                        fontSize: 10.5,
                        fontWeight: 700,
                        letterSpacing: '.08em',
                        textTransform: 'uppercase',
                    }}
                >
                    {style.label}
                    {notice.actor?.name ? ` · ${notice.actor.name}` : ''}
                </div>
                <div
                    style={{
                        fontWeight: 650,
                        fontSize: 14,
                        marginTop: 2,
                        whiteSpace: 'nowrap',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                    }}
                >
                    {notice.title}
                </div>
                {notice.body && (
                    <div
                        style={{
                            color: '#d9b99f',
                            marginTop: 2,
                            display: '-webkit-box',
                            WebkitLineClamp: 2,
                            WebkitBoxOrient: 'vertical',
                            overflow: 'hidden',
                        }}
                    >
                        {notice.body}
                    </div>
                )}
            </div>
            <button
                type="button"
                aria-label="Dismiss"
                onClick={(e) => {
                    e.stopPropagation()
                    void call('toast_dismiss', Nothing)
                }}
                style={{
                    position: 'absolute',
                    top: 6,
                    right: 8,
                    width: 22,
                    height: 22,
                    border: 0,
                    borderRadius: 6,
                    background: 'transparent',
                    color: '#d9b99f',
                    fontSize: 15,
                    cursor: 'pointer',
                }}
            >
                ×
            </button>
            <div
                aria-hidden
                style={{
                    position: 'absolute',
                    left: 0,
                    bottom: 0,
                    height: 3,
                    background: accent,
                    opacity: 0.8,
                    width: '100%',
                    transformOrigin: 'left',
                    animation: `gs-toast-life ${SHOW_MS}ms linear forwards`,
                    animationPlayState: paused ? 'paused' : 'running',
                }}
            />
            <style>{`@keyframes gs-toast-life{from{transform:scaleX(1)}to{transform:scaleX(0)}}`}</style>
        </div>
    )
}
