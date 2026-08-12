import { useCallback, useEffect, useState } from 'react'
import {
    FiAlertTriangle,
    FiArrowUp,
    FiCornerUpLeft,
    FiEye,
    FiEyeOff,
    FiFolder,
    FiHome,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import type { DirListingT, DirRootT } from '~/lib/ipc/schemas'

/**
 * The app's own folder chooser.
 *
 * Tauri's dialog plugin opens the platform's native picker, which on Linux is a
 * GTK file chooser — themed by the user's desktop, sized by it, and the single
 * largest piece of GTK on screen once the app draws its own window frame. This
 * replaces it with a list the app renders itself, so the picker looks the same
 * on all five targets.
 *
 * It shows DIRECTORIES ONLY, because that is all the Rust side will return: see
 * the module header on `src-tauri/src/commands/fs.rs`, which sets out exactly
 * what listing the tree from the webview does and does not widen. Nothing here
 * can read a file, and there is no code path that asks it to.
 */
export default function FolderPicker({
    title,
    hint,
    initial,
    onPick,
    onCancel,
}: {
    title: string
    hint?: string
    /** Where to open. `null` starts at the user's home directory. */
    initial?: string | null
    onPick: (path: string) => void
    onCancel: () => void
}) {
    const [listing, setListing] = useState<DirListingT | null>(null)
    const [roots, setRoots] = useState<DirRootT[]>([])
    const [hidden, setHidden] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [busy, setBusy] = useState(true)

    const go = useCallback(async (path: string | null, showHidden: boolean) => {
        setBusy(true)
        setError(null)

        try {
            setListing(await ipc.fsListDirs(path, showHidden))
        } catch (err) {
            /*
             * A folder that has been deleted or unmounted since it was
             * saved is the ordinary case here, not an exceptional one. The
             * message stays and the previous listing stays with it, so the
             * user can go back up rather than being left in an empty box.
             */
            setError(
                err instanceof Error ? err.message : 'Could not open that folder.'
            )
        } finally {
            setBusy(false)
        }
    }, [])

    useEffect(() => {
        void ipc
            .fsRoots()
            .then(setRoots)
            .catch(() => setRoots([]))
    }, [])

    useEffect(() => {
        void go(initial ?? null, hidden)
        // `initial` is the STARTING point; re-navigating because the prop was
        // rebuilt would yank the user back to it mid-browse.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [hidden])

    // Escape closes, which is what every native picker does and the one
    // keyboard affordance a modal cannot do without.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') onCancel()
        }

        window.addEventListener('keydown', onKey)

        return () => window.removeEventListener('keydown', onKey)
    }, [onCancel])

    return (
        <div
            className="fixed inset-0 z-[60] flex items-center justify-center bg-black/50 p-4 backdrop-blur-sm"
            onClick={onCancel}
        >
            <div
                onClick={(e) => e.stopPropagation()}
                role="dialog"
                aria-modal="true"
                aria-label={title}
                className="flex max-h-[80vh] w-full max-w-lg flex-col overflow-hidden rounded-xl border border-border bg-surface shadow-xl"
            >
                <header className="flex flex-col gap-1 border-b border-border px-4 py-3">
                    <h2 className="text-sm font-semibold">{title}</h2>
                    {hint && <p className="text-xs text-muted">{hint}</p>}
                </header>

                {/* ------------------------------------------------- Shortcuts */}
                {roots.length > 0 && (
                    <div className="flex gap-1 overflow-x-auto border-b border-border px-3 py-2">
                        {roots.map((root) => (
                            <button
                                key={root.path}
                                type="button"
                                onClick={() => void go(root.path, hidden)}
                                className="flex shrink-0 items-center gap-1.5 rounded-full bg-surface-secondary px-2.5 py-1 text-xs text-muted transition-colors hover:text-foreground"
                            >
                                <FiHome className="size-3" />
                                {root.label}
                            </button>
                        ))}
                    </div>
                )}

                {/* ----------------------------------------------------- Crumb */}
                <div className="flex items-center gap-2 border-b border-border px-3 py-2">
                    <button
                        type="button"
                        disabled={!listing?.parent}
                        onClick={() =>
                            listing?.parent && void go(listing.parent, hidden)
                        }
                        aria-label="Up one folder"
                        title="Up one folder"
                        className="rounded-lg p-1.5 text-muted transition-colors hover:bg-surface-hover hover:text-foreground disabled:opacity-30 disabled:hover:bg-transparent"
                    >
                        <FiArrowUp className="size-4" />
                    </button>

                    <span
                        className="selectable min-w-0 flex-1 truncate text-xs text-muted"
                        dir="rtl"
                        title={listing?.path}
                    >
                        {listing?.path ?? '…'}
                    </span>

                    <button
                        type="button"
                        onClick={() => setHidden((on) => !on)}
                        aria-label={
                            hidden ? 'Hide hidden folders' : 'Show hidden folders'
                        }
                        title={
                            hidden ? 'Hide hidden folders' : 'Show hidden folders'
                        }
                        aria-pressed={hidden}
                        className="rounded-lg p-1.5 text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
                    >
                        {hidden ? (
                            <FiEye className="size-4" />
                        ) : (
                            <FiEyeOff className="size-4" />
                        )}
                    </button>
                </div>

                {/* ----------------------------------------------------- List */}
                <div className="min-h-0 flex-1 overflow-y-auto">
                    {error && (
                        <p className="flex items-center gap-2 px-4 py-3 text-xs text-danger">
                            <FiCornerUpLeft className="size-3.5 shrink-0" />
                            {error}
                        </p>
                    )}

                    {listing?.denied && (
                        <p className="flex items-center gap-2 px-4 py-3 text-xs text-warning">
                            <FiAlertTriangle className="size-3.5 shrink-0" />
                            This folder cannot be listed on this account.
                        </p>
                    )}

                    {busy && !listing && (
                        <p className="px-4 py-6 text-center text-xs text-muted">
                            Loading…
                        </p>
                    )}

                    {listing?.entries.map((entry) => (
                        <button
                            key={entry.path}
                            type="button"
                            onClick={() => void go(entry.path, hidden)}
                            className="flex w-full items-center gap-2 px-4 py-2 text-left text-sm transition-colors hover:bg-surface-hover"
                        >
                            <FiFolder className="size-4 shrink-0 text-muted" />
                            <span className="truncate">{entry.name}</span>
                        </button>
                    ))}

                    {listing && listing.entries.length === 0 && !listing.denied && (
                        <p className="px-4 py-6 text-center text-xs text-muted">
                            No folders in here.
                        </p>
                    )}

                    {listing?.truncated && (
                        <p className="px-4 py-2 text-center text-[0.7rem] text-muted">
                            Only the first folders are shown — this one has too many
                            to list.
                        </p>
                    )}
                </div>

                {/* --------------------------------------------------- Actions */}
                <footer className="flex items-center justify-end gap-2 border-t border-border px-4 py-3">
                    <button
                        type="button"
                        onClick={onCancel}
                        className="rounded-lg px-3 py-1.5 text-sm text-muted transition-colors hover:text-foreground"
                    >
                        Cancel
                    </button>
                    <button
                        type="button"
                        disabled={!listing}
                        onClick={() => listing && onPick(listing.path)}
                        className="rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground disabled:opacity-40"
                    >
                        {/* The button picks the folder the user is STANDING IN,
                            not one highlighted in the list. That is how every
                            native directory picker behaves, and it removes the
                            "select then confirm" step that makes people choose
                            the parent by mistake. */}
                        Use this folder
                    </button>
                </footer>
            </div>
        </div>
    )
}

type PickOptions = {
    title: string
    hint?: string
    /** Where to open. Pass the current value so re-picking starts where the
     *  user left off rather than at home. */
    initial?: string | null
}

/**
 * The picker as an awaitable call, so the call sites read the way the native
 * dialog did: `const dir = await pick({ title })`, resolving to `null` when the
 * user backs out.
 *
 * Returns the element to render as well, because a modal has to live in the
 * tree — there is no `window.showDirectoryPicker` to hide behind here, and a
 * portal to `document.body` would escape the theme class the settings screens
 * sit under.
 */
export function useFolderPicker() {
    const [request, setRequest] = useState<
        (PickOptions & { resolve: (path: string | null) => void }) | null
    >(null)

    const pick = useCallback(
        (options: PickOptions) =>
            new Promise<string | null>((resolve) => {
                setRequest({ ...options, resolve })
            }),
        []
    )

    const settle = useCallback((path: string | null) => {
        setRequest((open) => {
            open?.resolve(path)

            return null
        })
    }, [])

    const element = request ? (
        <FolderPicker
            title={request.title}
            hint={request.hint}
            initial={request.initial}
            onPick={settle}
            onCancel={() => settle(null)}
        />
    ) : null

    return { pick, element }
}
