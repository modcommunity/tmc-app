import type { ReactNode } from 'react'

/** Small shared primitives so every settings pane looks the same. */

export function Section({
    title,
    hint,
    children,
}: {
    title: string
    hint?: string
    children: ReactNode
}) {
    return (
        <section className="flex flex-col gap-2">
            <div>
                <h2 className="text-sm font-semibold">{title}</h2>
                {hint && <p className="text-xs text-muted">{hint}</p>}
            </div>
            <div className="flex flex-col divide-y divide-border overflow-hidden rounded-xl border border-border bg-surface">
                {children}
            </div>
        </section>
    )
}

export function Row({
    label,
    hint,
    control,
}: {
    label: string
    hint?: string
    control: ReactNode
}) {
    return (
        <div className="flex items-center justify-between gap-4 px-3 py-2.5">
            <div className="min-w-0">
                <p className="text-sm">{label}</p>
                {hint && <p className="text-xs text-muted">{hint}</p>}
            </div>
            <div className="shrink-0">{control}</div>
        </div>
    )
}

/**
 * A switch built from a real checkbox.
 *
 * The visual is `peer-checked:` styling over a visually-hidden input, so it
 * keeps keyboard focus, screen-reader semantics and form behaviour that a
 * `<div onClick>` would silently drop.
 */
export function Toggle({
    checked,
    onChange,
    disabled,
    label,
}: {
    checked: boolean
    onChange: (next: boolean) => void
    disabled?: boolean
    label: string
}) {
    return (
        <label className="relative inline-flex cursor-pointer items-center">
            <input
                type="checkbox"
                role="switch"
                aria-label={label}
                className="peer sr-only"
                checked={checked}
                disabled={disabled}
                onChange={(e) => onChange(e.target.checked)}
            />
            <span className="h-6 w-11 rounded-full bg-surface-tertiary transition-colors peer-checked:bg-accent peer-disabled:opacity-50" />
            <span className="absolute left-0.5 size-5 rounded-full bg-background transition-transform peer-checked:translate-x-5" />
        </label>
    )
}

/**
 * Re-exported from `components/select`, which is a real listbox rather than a
 * `<select>`.
 *
 * The native element's popup is drawn by the OS and cannot be styled at all, so
 * every settings pane in a dark theme had one white rectangle in it. See that
 * module's header for what replacing it costs and what it buys.
 */
export { default as Select } from '~/components/select'
export type { SelectOption } from '~/components/select'
