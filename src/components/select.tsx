import {
    useCallback,
    useEffect,
    useId,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
} from 'react'
import { Check, ChevronDown } from 'lucide-react'

/**
 * A dropdown the app actually owns.
 *
 * WHY NOT `<select>`
 * ------------------
 * Because a native `<select>`'s POPUP is drawn by the operating system and
 * nothing in CSS reaches it. The closed control can be styled; the open list
 * cannot. That produces exactly the mismatch the custom titlebar exists to
 * remove — a GTK list on Linux, an Aqua one on macOS, a Fluent one on Windows —
 * and in dark mode it is worse than a mismatch, because two of the three ignore
 * the app's palette entirely and render dark text on white while everything
 * around them is dark.
 *
 * It also cannot hold an icon, and a game picker without game icons is a wall
 * of text.
 *
 * WHAT THAT COSTS, AND WHAT IS PAID BACK
 * --------------------------------------
 * Everything a native control gives for free has to be re-implemented, and the
 * usual failure of a hand-rolled dropdown is that half of it is not:
 *
 *   * **Keyboard.** Arrows move, Home/End jump, Enter and Space commit, Escape
 *     closes, Tab closes and moves on. Typing letters jumps to a match, which
 *     is the one native behaviour people use without noticing.
 *   * **Screen readers.** A real `combobox`/`listbox` pair with
 *     `aria-activedescendant`, so the focus never actually leaves the button
 *     and the reader still announces the highlighted row.
 *   * **Dismissal.** Pointer-down outside, not click — a click that starts
 *     inside and ends outside is a drag, not a dismissal.
 *   * **Placement.** Flipped above the button when there is no room below,
 *     which on a phone-sized window is most of the time.
 *
 * MOBILE
 * ------
 * The list is a normal absolutely-positioned element rather than a sheet. This
 * app's mobile shell is the same document at a narrower width, and a control
 * that became a modal below 768px would be a second thing to keep working.
 */

export type SelectOption<T extends string> = {
    value: T
    label: string
    /** Second line, for the cases where the label alone is ambiguous. */
    hint?: string
    /** An icon, a game's own artwork, a colour swatch. */
    icon?: ReactNode
    disabled?: boolean
    /** Renders a heading above this option. */
    group?: string
}

type Props<T extends string> = {
    value: T
    options: SelectOption<T>[]
    onChange: (next: T) => void
    /** Accessible name. Required — a picker nobody can name is one nobody can use. */
    label: string
    /** Shown when `value` matches no option. */
    placeholder?: string
    disabled?: boolean
    className?: string
    /** Match the width of the button rather than sizing to content. */
    fullWidth?: boolean
}

export default function Select<T extends string>({
    value,
    options,
    onChange,
    label,
    placeholder = 'Select…',
    disabled,
    className = '',
    fullWidth,
}: Props<T>) {
    const [open, setOpen] = useState(false)
    const [active, setActive] = useState(0)
    const [above, setAbove] = useState(false)

    const rootRef = useRef<HTMLDivElement>(null)
    const buttonRef = useRef<HTMLButtonElement>(null)
    const listRef = useRef<HTMLDivElement>(null)

    /*
     * Type-ahead state. A ref rather than state: it changes on every keystroke
     * and nothing renders from it, so putting it in state would be a re-render
     * per character for no visible difference.
     */
    const typed = useRef({ text: '', at: 0 })

    const id = useId()

    const selectable = useMemo(
        () => options.filter((option) => !option.disabled),
        [options]
    )

    const selected = options.find((option) => option.value === value)

    const indexOfValue = useCallback(
        () =>
            Math.max(
                0,
                options.findIndex((option) => option.value === value)
            ),
        [options, value]
    )

    // ------------------------------------------------------------ Dismissal
    useEffect(() => {
        if (!open) return

        const onPointerDown = (event: PointerEvent) => {
            if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
        }

        // `pointerdown`, not `click`: a click that starts inside the list and
        // ends outside it is a drag, and closing on that loses the selection
        // somebody was in the middle of making.
        document.addEventListener('pointerdown', onPointerDown)

        return () => document.removeEventListener('pointerdown', onPointerDown)
    }, [open])

    // ------------------------------------------------------------ Placement
    useLayoutEffect(() => {
        if (!open) return

        const button = buttonRef.current

        if (!button) return

        const rect = button.getBoundingClientRect()
        const below = window.innerHeight - rect.bottom

        // 260px is the list's max height. Flipping matters most on the mobile
        // shell, where a control near the bottom has nowhere to open.
        setAbove(below < 260 && rect.top > below)
    }, [open])

    // Keep the highlighted row in view while arrowing through a long list.
    useEffect(() => {
        if (!open) return

        listRef.current
            ?.querySelector(`[data-index="${active}"]`)
            ?.scrollIntoView({ block: 'nearest' })
    }, [open, active])

    const commit = (option: SelectOption<T>) => {
        if (option.disabled) return

        onChange(option.value)
        setOpen(false)
        buttonRef.current?.focus()
    }

    const move = (delta: number) => {
        if (!selectable.length) return

        setActive((current) => {
            let next = current

            for (let step = 0; step < options.length; step += 1) {
                next = (next + delta + options.length) % options.length

                if (!options[next]?.disabled) return next
            }

            return current
        })
    }

    const onKeyDown = (event: React.KeyboardEvent) => {
        switch (event.key) {
            case 'ArrowDown':
            case 'ArrowUp': {
                event.preventDefault()

                if (!open) {
                    setActive(indexOfValue())
                    setOpen(true)

                    return
                }

                move(event.key === 'ArrowDown' ? 1 : -1)

                return
            }
            case 'Home':
            case 'End': {
                if (!open) return

                event.preventDefault()
                setActive(event.key === 'Home' ? 0 : options.length - 1)

                return
            }
            case 'Enter':
            case ' ': {
                event.preventDefault()

                if (!open) {
                    setActive(indexOfValue())
                    setOpen(true)

                    return
                }

                const option = options[active]

                if (option) commit(option)

                return
            }
            case 'Escape': {
                if (!open) return

                event.preventDefault()
                setOpen(false)

                return
            }
            case 'Tab': {
                // Closes and moves on, as a native select does. Not prevented —
                // the focus should actually go to the next control.
                setOpen(false)

                return
            }
            default:
                break
        }

        /*
         * Type-ahead. A single printable character, with a one-second window in
         * which further characters extend the search rather than restart it —
         * so "mi", "min" and "mine" all reach Minecraft while "m" then a pause
         * then "m" cycles through the entries beginning with M.
         */
        if (event.key.length !== 1 || event.ctrlKey || event.metaKey) return

        const now = Date.now()

        typed.current.text =
            now - typed.current.at > 1000
                ? event.key.toLowerCase()
                : typed.current.text + event.key.toLowerCase()

        typed.current.at = now

        const query = typed.current.text

        const from = open ? active : indexOfValue()

        // Search from the row AFTER the current one, wrapping, so repeating a
        // letter steps through matches instead of sticking on the first.
        for (let step = 1; step <= options.length; step += 1) {
            const index = (from + step) % options.length
            const option = options[index]

            if (
                option &&
                !option.disabled &&
                option.label.toLowerCase().startsWith(query)
            ) {
                setActive(index)

                if (!open) commit(option)

                return
            }
        }
    }

    let lastGroup: string | undefined

    return (
        <div
            ref={rootRef}
            className={`relative ${fullWidth ? 'w-full' : 'inline-block'} ${className}`}
        >
            <button
                ref={buttonRef}
                type="button"
                role="combobox"
                aria-expanded={open}
                aria-haspopup="listbox"
                aria-controls={`${id}-list`}
                aria-label={label}
                aria-activedescendant={open ? `${id}-opt-${active}` : undefined}
                disabled={disabled}
                onClick={() => {
                    if (disabled) return

                    setActive(indexOfValue())
                    setOpen((was) => !was)
                }}
                onKeyDown={onKeyDown}
                className={`flex ${
                    fullWidth ? 'w-full' : ''
                } items-center gap-2 rounded-lg border border-border bg-surface px-2 py-1.5 text-left text-sm transition-colors hover:border-accent focus:border-accent focus:outline-none disabled:cursor-not-allowed disabled:opacity-50`}
            >
                {selected?.icon && (
                    <span className="flex size-4 shrink-0 items-center justify-center">
                        {selected.icon}
                    </span>
                )}

                <span
                    className={`min-w-0 flex-1 truncate ${
                        selected ? '' : 'text-muted'
                    }`}
                >
                    {selected?.label ?? placeholder}
                </span>

                <ChevronDown
                    aria-hidden
                    className={`size-3.5 shrink-0 text-muted transition-transform ${
                        open ? 'rotate-180' : ''
                    }`}
                />
            </button>

            {open && (
                <div
                    ref={listRef}
                    id={`${id}-list`}
                    role="listbox"
                    aria-label={label}
                    className={`absolute z-50 max-h-[260px] min-w-full overflow-y-auto rounded-lg border border-border bg-surface p-1 shadow-lg ${
                        above ? 'bottom-full mb-1' : 'top-full mt-1'
                    }`}
                >
                    {options.length === 0 && (
                        <p className="px-2 py-3 text-center text-xs text-muted">
                            Nothing to choose from
                        </p>
                    )}

                    {options.map((option, index) => {
                        const heading =
                            option.group && option.group !== lastGroup
                                ? option.group
                                : undefined

                        lastGroup = option.group

                        return (
                            <div key={option.value}>
                                {heading && (
                                    <p className="px-2 pb-1 pt-2 text-[0.65rem] font-semibold uppercase tracking-wide text-muted">
                                        {heading}
                                    </p>
                                )}

                                <div
                                    id={`${id}-opt-${index}`}
                                    data-index={index}
                                    role="option"
                                    aria-selected={option.value === value}
                                    aria-disabled={option.disabled}
                                    /*
                                     * `pointerdown`, not `click`: the document
                                     * listener above closes on pointerdown, and
                                     * a click handler here would never fire
                                     * because the list is gone before the
                                     * pointer comes back up.
                                     */
                                    onPointerDown={(event) => {
                                        event.preventDefault()
                                        commit(option)
                                    }}
                                    onPointerEnter={() =>
                                        !option.disabled && setActive(index)
                                    }
                                    className={`flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm ${
                                        option.disabled
                                            ? 'cursor-not-allowed opacity-40'
                                            : index === active
                                              ? 'bg-surface-tertiary'
                                              : ''
                                    }`}
                                >
                                    {option.icon && (
                                        <span className="flex size-4 shrink-0 items-center justify-center">
                                            {option.icon}
                                        </span>
                                    )}

                                    <span className="min-w-0 flex-1">
                                        <span className="block truncate">
                                            {option.label}
                                        </span>
                                        {option.hint && (
                                            <span className="block truncate text-[0.7rem] text-muted">
                                                {option.hint}
                                            </span>
                                        )}
                                    </span>

                                    {option.value === value && (
                                        <Check
                                            aria-hidden
                                            className="size-3.5 shrink-0 text-accent"
                                        />
                                    )}
                                </div>
                            </div>
                        )
                    })}
                </div>
            )}
        </div>
    )
}
