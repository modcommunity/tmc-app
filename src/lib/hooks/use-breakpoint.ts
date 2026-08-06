import { useSyncExternalStore } from 'react'

/**
 * Layout decisions keyed on the WINDOW, not the platform.
 *
 * A tablet in portrait, a phone, and a desktop window dragged narrow all want
 * the same layout, and none of them are distinguishable by `os.platform()`.
 * Driving off the viewport means one code path serves all five targets and the
 * mobile shell is testable by resizing a window.
 *
 * `useSyncExternalStore` rather than a `useState` + `resize` listener: it gives
 * the correct value on the very first render, so the shell does not paint the
 * desktop rail for a frame before switching to tabs.
 */
function subscribe(query: string) {
    return (onChange: () => void) => {
        const media = window.matchMedia(query)

        media.addEventListener('change', onChange)

        return () => media.removeEventListener('change', onChange)
    }
}

export function useMediaQuery(query: string): boolean {
    return useSyncExternalStore(
        subscribe(query),
        () => window.matchMedia(query).matches,
        // Server snapshot. Never hit in a Tauri build, but React demands it and
        // "not compact" is the safer default for a static render.
        () => false
    )
}

/** Phone / narrow-window layout: bottom tabs, single-column lists. */
export function useIsCompact(): boolean {
    return useMediaQuery('(max-width: 767px)')
}

/** Wide enough for a filter sidebar next to the grid. */
export function useIsWide(): boolean {
    return useMediaQuery('(min-width: 1100px)')
}

/** Coarse pointer: bigger hit targets, no hover-only affordances. */
export function useIsTouch(): boolean {
    return useMediaQuery('(pointer: coarse)')
}
