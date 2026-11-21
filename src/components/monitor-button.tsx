import { useState } from 'react'
import { FiActivity } from 'react-icons/fi'

import { openMonitor } from '~/lib/monitor-window'
import type { MonitorTarget } from '~/lib/monitor'

type Variant = 'compact' | 'icon' | 'full'

const STYLES: Record<Variant, string> = {
    compact:
        'gap-1.5 rounded-md border border-border bg-surface px-2.5 py-1 text-xs font-medium',
    icon: 'size-7 justify-center rounded-md border border-border bg-background/80 backdrop-blur',
    full: 'gap-2 rounded-lg border border-border bg-surface px-4 py-2 text-sm font-medium',
}

/**
 * Opens the server's monitor window.
 *
 * The sibling of Join: same height and radius, but outlined rather than filled
 * so there is one obvious primary action per row. Every click stops
 * propagation, because the table row and the card both navigate on click.
 */
export default function MonitorButton({
    target,
    variant = 'compact',
    className = '',
}: {
    target: MonitorTarget
    variant?: Variant
    className?: string
}) {
    const [failed, setFailed] = useState(false)

    return (
        <button
            type="button"
            title={failed ? 'Could not open the monitor window' : 'Monitor — open live latency in its own window'}
            aria-label={`Monitor ${target.name ?? 'server'}`}
            onClick={(e) => {
                e.preventDefault()
                e.stopPropagation()
                setFailed(false)
                openMonitor(target).catch((err) => {
                    console.error('[monitor] could not open window', err)
                    setFailed(true)
                })
            }}
            className={`inline-flex items-center transition-colors hover:border-accent hover:text-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent ${
                failed ? 'border-danger text-danger' : ''
            } ${STYLES[variant]} ${className}`}
        >
            <FiActivity className={variant === 'full' ? 'size-4' : 'size-3.5'} />
            {variant !== 'icon' && <span>Monitor</span>}
        </button>
    )
}
