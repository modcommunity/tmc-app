/**
 * Command history for the Console, kept in localStorage.
 *
 * What is stored is the DISPLAY form Rust returns — credential-shaped
 * arguments already redacted — never what was typed, so a token pasted into
 * `auth login --token …` is not written to disk by a convenience feature.
 */

export const HISTORY_KEY = 'tmc.console.history'
export const MAX_HISTORY = 100

/** Newest last. A repeat of the previous entry is not added twice. */
export function pushHistory(list: string[], line: string): string[] {
    const trimmed = line.trim()

    if (!trimmed || list[list.length - 1] === trimmed) return list

    return [...list, trimmed].slice(-MAX_HISTORY)
}

/**
 * Step through history. `cursor` is an index into `list`, or `list.length` for
 * "the line being typed". Returns the new cursor and the text to show.
 */
export function walkHistory(
    list: string[],
    cursor: number,
    direction: 'up' | 'down',
    draft: string
): { cursor: number; text: string } {
    const next =
        direction === 'up'
            ? Math.max(0, cursor - 1)
            : Math.min(list.length, cursor + 1)

    return {
        cursor: next,
        text: next >= list.length ? draft : (list[next] ?? draft),
    }
}

export function loadHistory(): string[] {
    try {
        const raw: unknown = JSON.parse(localStorage.getItem(HISTORY_KEY) ?? '[]')

        return Array.isArray(raw)
            ? raw
                  .filter((x): x is string => typeof x === 'string')
                  .slice(-MAX_HISTORY)
            : []
    } catch {
        return []
    }
}

export function saveHistory(list: string[]): void {
    try {
        localStorage.setItem(HISTORY_KEY, JSON.stringify(list))
    } catch {
        // Private window or blocked storage: history lasts until the screen closes.
    }
}
