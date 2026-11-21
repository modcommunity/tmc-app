import { useEffect, useRef, type RefObject } from 'react'

/**
 * Behaviour every modal dialog shares: focus moves in when it opens, Tab stays
 * inside it, Escape closes it, and focus goes back to whatever opened it.
 *
 * Dialogs can nest (a confirmation inside the play dialog), so a stack decides
 * which one owns the keyboard: only the top-most reacts to Escape and Tab.
 *
 * Usage: put `ref` on the element with `role="dialog"` and give it
 * `tabIndex={-1}`; the first focusable control inside gets focus, or the dialog
 * itself when it has none. `autoFocus` on a child is honoured.
 */
const stack: symbol[] = []

const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

function focusables(root: HTMLElement): HTMLElement[] {
    return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
        (el) => !el.hasAttribute('inert') && el.getClientRects().length > 0,
    )
}

export function useDialog<T extends HTMLElement>(onClose: () => void): RefObject<T | null> {
    const ref = useRef<T>(null)
    const close = useRef(onClose)

    useEffect(() => {
        close.current = onClose
    })

    useEffect(() => {
        const id = Symbol('dialog')
        const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null

        stack.push(id)

        const initial = ref.current

        if (initial && !initial.contains(document.activeElement)) {
            ;(focusables(initial)[0] ?? initial).focus()
        }

        const onKey = (e: KeyboardEvent) => {
            const root = ref.current

            if (stack[stack.length - 1] !== id || !root) return

            if (e.key === 'Escape') {
                e.preventDefault()
                e.stopPropagation()
                close.current()

                return
            }

            if (e.key !== 'Tab') return

            const items = focusables(root)

            if (items.length === 0) {
                e.preventDefault()
                root.focus()

                return
            }

            const first = items[0]!
            const last = items[items.length - 1]!
            const active = document.activeElement

            if (e.shiftKey && (active === first || active === root || !root.contains(active))) {
                e.preventDefault()
                last.focus()
            } else if (!e.shiftKey && (active === last || !root.contains(active))) {
                e.preventDefault()
                first.focus()
            }
        }

        document.addEventListener('keydown', onKey, true)

        return () => {
            document.removeEventListener('keydown', onKey, true)

            const at = stack.indexOf(id)

            if (at >= 0) stack.splice(at, 1)

            if (opener?.isConnected) opener.focus()
        }
    }, [])

    return ref
}
