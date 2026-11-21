import { useCallback, useEffect, useId, useRef, useState } from 'react'
import {
    FiAlertTriangle,
    FiEdit2,
    FiFolder,
    FiHardDrive,
    FiPlus,
    FiRefreshCw,
    FiStar,
    FiTrash2,
} from 'react-icons/fi'

import { useFolderPicker } from '~/components/folder-picker'
import { formatBytes } from '~/lib/downloads/provider'
import { isIpcError } from '~/lib/ipc'
import { ipc } from '~/lib/ipc/commands'
import type { StoreKindT, StoreT } from '~/lib/ipc/schemas'
import { canBeDefault, storeNameProblem, usageFraction } from '~/lib/stores/logic'

/**
 * **Stores**: named folders downloads and installs can live in.
 *
 * A permanent store is kept; a temporary one is emptied when the app closes
 * (and again at the next launch, in case it crashed). Every rule that matters
 * (is the folder allowed, is it nested in another store, is a game installed
 * in it) is enforced in Rust; this screen shows Rust's refusal instead of
 * pre-judging it, apart from the cheap name checks.
 */
export default function StoresRoute() {
    const [stores, setStores] = useState<StoreT[] | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [creating, setCreating] = useState(false)
    const [renaming, setRenaming] = useState<StoreT | null>(null)
    const [deleting, setDeleting] = useState<StoreT | null>(null)

    const load = useCallback(async () => {
        try {
            setStores(await ipc.storageList())
            setError(null)
        } catch (err) {
            setError(message(err, 'Could not read your stores.'))
        }
    }, [])

    useEffect(() => {
        queueMicrotask(() => void load())
    }, [load])

    const act = async (work: () => Promise<void>) => {
        setError(null)

        try {
            await work()
            await load()
        } catch (err) {
            setError(message(err, 'That did not work.'))
        }
    }

    return (
        <div className="mx-auto flex w-full max-w-4xl flex-col gap-4 p-4">
            <header className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 flex-1 basis-64">
                    <h1 className="text-lg font-bold">Stores</h1>
                    <p className="mt-1 text-sm text-muted">
                        Named folders for downloads and installs. Permanent
                        stores are kept; temporary ones are emptied when the app
                        closes.
                    </p>
                </div>

                <button
                    type="button"
                    onClick={() => setCreating(true)}
                    className="flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-white transition-opacity hover:opacity-90 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                    <FiPlus aria-hidden className="h-4 w-4" />
                    Create store
                </button>
            </header>

            {error ? (
                <p
                    role="alert"
                    className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger"
                >
                    <FiAlertTriangle aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                    <span className="min-w-0 flex-1">{error}</span>
                    {stores === null ? (
                        <button
                            type="button"
                            onClick={() => void load()}
                            className="flex shrink-0 items-center gap-1 rounded-md border border-danger/40 px-2 py-0.5 font-medium hover:bg-danger/10"
                        >
                            <FiRefreshCw aria-hidden className="h-3 w-3" />
                            Retry
                        </button>
                    ) : null}
                </p>
            ) : null}

            {stores === null ? (
                error ? null : (
                    <div role="status" aria-label="Loading stores" className="flex flex-col gap-3">
                        {[0, 1].map((i) => (
                            <div
                                key={i}
                                className="h-24 rounded-xl border border-border bg-surface motion-safe:animate-pulse"
                            />
                        ))}
                    </div>
                )
            ) : stores.length === 0 ? (
                <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border p-8 text-center">
                    <FiHardDrive aria-hidden className="h-6 w-6 text-muted" />
                    <p className="max-w-sm text-sm text-muted">
                        No stores yet. Create one to choose where downloads go.
                    </p>
                </div>
            ) : (
                <ul className="flex flex-col gap-3" aria-label="Stores">
                    {stores.map((store) => (
                        <StoreCard
                            key={store.id}
                            store={store}
                            onDefault={() => void act(() => ipc.storageSetDefault(store.id))}
                            onRename={() => setRenaming(store)}
                            onDelete={() => setDeleting(store)}
                        />
                    ))}
                </ul>
            )}

            {creating ? (
                <CreateDialog
                    existing={(stores ?? []).map((s) => s.name)}
                    onClose={() => setCreating(false)}
                    onCreated={() => {
                        setCreating(false)
                        void load()
                    }}
                />
            ) : null}

            {renaming ? (
                <RenameDialog
                    store={renaming}
                    existing={(stores ?? []).filter((s) => s.id !== renaming.id).map((s) => s.name)}
                    onClose={() => setRenaming(null)}
                    onDone={() => {
                        setRenaming(null)
                        void load()
                    }}
                />
            ) : null}

            {deleting ? (
                <DeleteDialog
                    store={deleting}
                    onClose={() => setDeleting(null)}
                    onDone={() => {
                        setDeleting(null)
                        void load()
                    }}
                />
            ) : null}
        </div>
    )
}

function message(err: unknown, fallback: string): string {
    return isIpcError(err) ? err.message : fallback
}

function Badge({ children, tone }: { children: string; tone: 'accent' | 'muted' }) {
    return (
        <span
            className={`rounded-full border px-2 py-0.5 text-[0.65rem] font-medium ${
                tone === 'accent'
                    ? 'border-accent text-accent'
                    : 'border-border text-muted'
            }`}
        >
            {children}
        </span>
    )
}

function StoreCard({
    store,
    onDefault,
    onRename,
    onDelete,
}: {
    store: StoreT
    onDefault: () => void
    onRename: () => void
    onDelete: () => void
}) {
    const fraction = usageFraction(store)
    const used = `${formatBytes(store.usedBytes)}${store.usedTruncated ? '+' : ''}`

    return (
        <li
            className={`rounded-xl border bg-surface p-3 ${
                store.exists ? 'border-border' : 'border-danger/50'
            }`}
        >
            <div className="flex flex-wrap items-center gap-2">
                <FiFolder aria-hidden className="h-4 w-4 shrink-0 text-muted" />
                <h2 className="min-w-0 break-words text-sm font-semibold">{store.name}</h2>
                <Badge tone="muted">{store.kind === 'temporary' ? 'Temporary' : 'Permanent'}</Badge>
                {store.isDefault ? <Badge tone="accent">Default</Badge> : null}
                {store.builtin ? <Badge tone="muted">Built in</Badge> : null}
            </div>

            <p className="selectable mt-1 break-all text-[11px] text-muted">{store.path}</p>

            {store.exists ? (
                <div className="mt-2">
                    {fraction !== null ? (
                        <div
                            role="progressbar"
                            aria-label={`${store.name} disk usage`}
                            aria-valuemin={0}
                            aria-valuemax={100}
                            aria-valuenow={Math.round(fraction * 100)}
                            className="h-1.5 overflow-hidden rounded-full bg-surface-tertiary"
                        >
                            <div
                                className="h-full rounded-full bg-accent"
                                style={{ width: `${Math.max(fraction * 100, fraction > 0 ? 1 : 0)}%` }}
                            />
                        </div>
                    ) : null}

                    <p className="mt-1 text-[11px] text-muted">
                        {used} used
                        {store.freeBytes !== null ? ` · ${formatBytes(store.freeBytes)} free` : ''}
                    </p>
                </div>
            ) : (
                <p className="mt-2 text-[11px] text-danger">
                    This folder is missing. Is the drive connected?
                </p>
            )}

            <div className="mt-3 flex flex-wrap gap-2">
                {canBeDefault(store) ? (
                    <CardButton onClick={onDefault} icon={<FiStar aria-hidden className="h-3 w-3" />}>
                        Set as default
                    </CardButton>
                ) : null}

                {store.builtin ? null : (
                    <>
                        <CardButton onClick={onRename} icon={<FiEdit2 aria-hidden className="h-3 w-3" />}>
                            Rename
                        </CardButton>
                        <CardButton
                            onClick={onDelete}
                            icon={<FiTrash2 aria-hidden className="h-3 w-3" />}
                        >
                            Delete
                        </CardButton>
                    </>
                )}
            </div>
        </li>
    )
}

function CardButton({
    children,
    icon,
    onClick,
}: {
    children: string
    icon: React.ReactNode
    onClick: () => void
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            className="flex items-center gap-1.5 rounded-lg border border-border px-2 py-1 text-xs transition-colors hover:border-accent hover:text-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
        >
            {icon}
            {children}
        </button>
    )
}

/** Modal frame: focus moves in, Escape closes, labelled by its title. */
function Modal({
    title,
    onClose,
    children,
}: {
    title: string
    onClose: () => void
    children: React.ReactNode
}) {
    const titleId = useId()
    const ref = useRef<HTMLDivElement>(null)

    useEffect(() => {
        const previous = document.activeElement as HTMLElement | null

        ref.current?.querySelector<HTMLElement>('input, button')?.focus()

        return () => previous?.focus()
    }, [])

    return (
        <div
            className="fixed inset-0 z-50 flex items-center justify-center overflow-y-auto bg-black/60 p-4"
            onMouseDown={(e) => {
                if (e.target === e.currentTarget) onClose()
            }}
            onKeyDown={(e) => {
                if (e.key === 'Escape') onClose()
            }}
        >
            <div
                ref={ref}
                role="dialog"
                aria-modal="true"
                aria-labelledby={titleId}
                className="max-h-full w-full max-w-md overflow-y-auto rounded-xl border border-border bg-surface p-4 shadow-xl"
            >
                <h2 id={titleId} className="text-sm font-semibold">
                    {title}
                </h2>
                {children}
            </div>
        </div>
    )
}

function ErrorLine({ text }: { text: string | null }) {
    return text ? (
        <p role="alert" className="mt-3 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
            {text}
        </p>
    ) : null
}

function Actions({
    onClose,
    submitLabel,
    busy,
    disabled,
    danger,
}: {
    onClose: () => void
    submitLabel: string
    busy: boolean
    disabled?: boolean
    danger?: boolean
}) {
    return (
        <div className="mt-4 flex justify-end gap-2">
            <button
                type="button"
                onClick={onClose}
                className="rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent"
            >
                Cancel
            </button>
            <button
                type="submit"
                disabled={busy || disabled}
                className={`rounded-lg px-3 py-1.5 text-xs font-medium text-white disabled:opacity-50 ${
                    danger ? 'bg-danger' : 'bg-accent'
                }`}
            >
                {busy ? 'Working...' : submitLabel}
            </button>
        </div>
    )
}

function CreateDialog({
    existing,
    onClose,
    onCreated,
}: {
    existing: string[]
    onClose: () => void
    onCreated: () => void
}) {
    const { pick, element: picker } = useFolderPicker()
    const [name, setName] = useState('')
    const [path, setPath] = useState<string | null>(null)
    const [kind, setKind] = useState<StoreKindT>('permanent')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const nameId = useId()
    const problem = name === '' ? null : storeNameProblem(name, existing)

    const submit = async (e: React.FormEvent) => {
        e.preventDefault()

        const bad = storeNameProblem(name, existing)

        if (bad || !path) {
            setError(bad ?? 'Choose a folder.')
            return
        }

        setBusy(true)
        setError(null)

        try {
            await ipc.storageCreate(name.trim(), path, kind)
            onCreated()
        } catch (err) {
            setError(message(err, 'Could not create the store.'))
            setBusy(false)
        }
    }

    return (
        <Modal title="Create store" onClose={onClose}>
            <form onSubmit={(e) => void submit(e)} className="mt-3 flex flex-col gap-3">
                <div>
                    <label htmlFor={nameId} className="text-xs font-medium">
                        Name
                    </label>
                    <input
                        id={nameId}
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        maxLength={64}
                        aria-invalid={problem !== null}
                        className="mt-1 w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-sm"
                    />
                    {problem ? <p className="mt-1 text-[11px] text-danger">{problem}</p> : null}
                </div>

                <div>
                    <span className="text-xs font-medium">Folder</span>
                    <div className="mt-1 flex items-center gap-2">
                        <button
                            type="button"
                            onClick={() =>
                                void pick({
                                    title: 'Choose the store folder',
                                    hint: 'It is created if it does not exist.',
                                    initial: path,
                                }).then((chosen) => {
                                    if (chosen) setPath(chosen)
                                })
                            }
                            className="flex shrink-0 items-center gap-1.5 rounded-lg border border-border px-2 py-1 text-xs hover:border-accent"
                        >
                            <FiFolder aria-hidden className="h-3 w-3" />
                            Browse...
                        </button>
                        <span className="selectable min-w-0 break-all text-[11px] text-muted">
                            {path ?? 'No folder chosen'}
                        </span>
                    </div>
                </div>

                <fieldset>
                    <legend className="text-xs font-medium">Kind</legend>
                    <div className="mt-1 flex flex-col gap-1.5 text-xs">
                        <label className="flex items-start gap-2">
                            <input
                                type="radio"
                                name="store-kind"
                                checked={kind === 'permanent'}
                                onChange={() => setKind('permanent')}
                            />
                            <span>Permanent: kept until you delete things yourself.</span>
                        </label>
                        <label className="flex items-start gap-2">
                            <input
                                type="radio"
                                name="store-kind"
                                checked={kind === 'temporary'}
                                onChange={() => setKind('temporary')}
                            />
                            <span>
                                Temporary: emptied when the app closes. Must be an empty folder.
                            </span>
                        </label>
                    </div>
                </fieldset>

                <ErrorLine text={error} />
                <Actions
                    onClose={onClose}
                    submitLabel="Create store"
                    busy={busy}
                    disabled={!name.trim() || !path}
                />
            </form>
            {picker}
        </Modal>
    )
}

function RenameDialog({
    store,
    existing,
    onClose,
    onDone,
}: {
    store: StoreT
    existing: string[]
    onClose: () => void
    onDone: () => void
}) {
    const [name, setName] = useState(store.name)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const nameId = useId()
    const problem = storeNameProblem(name, existing)

    const submit = async (e: React.FormEvent) => {
        e.preventDefault()

        if (problem) return

        setBusy(true)
        setError(null)

        try {
            await ipc.storageRename(store.id, name.trim())
            onDone()
        } catch (err) {
            setError(message(err, 'Could not rename the store.'))
            setBusy(false)
        }
    }

    return (
        <Modal title="Rename store" onClose={onClose}>
            <form onSubmit={(e) => void submit(e)} className="mt-3">
                <label htmlFor={nameId} className="text-xs font-medium">
                    Name
                </label>
                <input
                    id={nameId}
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    maxLength={64}
                    aria-invalid={problem !== null}
                    className="mt-1 w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-sm"
                />
                {problem ? <p className="mt-1 text-[11px] text-danger">{problem}</p> : null}
                <ErrorLine text={error} />
                <Actions onClose={onClose} submitLabel="Rename" busy={busy} disabled={problem !== null} />
            </form>
        </Modal>
    )
}

function DeleteDialog({
    store,
    onClose,
    onDone,
}: {
    store: StoreT
    onClose: () => void
    onDone: () => void
}) {
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const wipes = store.kind === 'temporary'
    const hasFiles = store.usedBytes > 0 || store.usedTruncated

    const submit = async (e: React.FormEvent) => {
        e.preventDefault()
        setBusy(true)
        setError(null)

        try {
            await ipc.storageDelete(store.id, wipes)
            onDone()
        } catch (err) {
            setError(message(err, 'Could not delete the store.'))
            setBusy(false)
        }
    }

    return (
        <Modal title={`Delete "${store.name}"?`} onClose={onClose}>
            <form onSubmit={(e) => void submit(e)}>
                <p className="mt-3 text-xs text-muted">
                    {wipes
                        ? hasFiles
                            ? 'This temporary store is not empty. Deleting it erases everything inside the folder.'
                            : 'The store is removed. The folder is left in place.'
                        : 'The store is only forgotten. The folder and every file in it stay where they are.'}
                </p>
                <p className="selectable mt-2 break-all text-[11px] text-muted">{store.path}</p>
                <ErrorLine text={error} />
                <Actions
                    onClose={onClose}
                    submitLabel={wipes && hasFiles ? 'Erase and delete' : 'Delete store'}
                    busy={busy}
                    danger
                />
            </form>
        </Modal>
    )
}
