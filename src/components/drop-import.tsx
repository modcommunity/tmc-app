import { useCallback, useEffect, useState } from 'react'
import { z } from 'zod'
import { FiDownload } from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { subscribe } from '~/lib/ipc'
import ImportDialog, { type ImportTarget } from '~/components/import-dialog'
import { DropBatchSchema, type SandboxRowT } from '~/lib/ipc/schemas'

/**
 * **Drag a mod onto the window and it is imported.**
 *
 * The affordance and the dialog, mounted once above the router so a drop works
 * on every screen rather than only on the one that thought to handle it.
 *
 * WHY THIS LISTENS TO RUST RATHER THAN TO THE DOM
 * -----------------------------------------------
 * `dragDropEnabled` is on in `tauri.conf.json`, which means the OS delivers the
 * drop to the WINDOW and the webview receives no HTML5 drag events at all.
 * That is the point: an `ondrop` handler here would be the untrusted half of
 * the app deciding what was dropped, and it would be handed real filesystem
 * paths to do it with. Instead Rust takes the paths, mints a token per file and
 * emits names — so this component never learns where anything is, and
 * `commands::import` is the only thing that can turn a token back into a path.
 *
 * WHY IT ALSO POLLS ONCE ON MOUNT
 * -------------------------------
 * An event fires whether or not anything is listening. A drop that lands while
 * the router is swapping a route would be silently lost, which reads as drag
 * and drop being broken rather than as a race — so Rust also holds the last
 * batch and this collects it.
 *
 * WHERE A DROP GOES
 * -----------------
 * Into the sandbox the user is looking at, when they are looking at one, and
 * otherwise into the device's imported list with the game left to be chosen.
 * `useDropTarget` is how a screen says which it is; without one the second is
 * the answer, and it is the safe one — a mod imported unattached is a row to
 * finish filling in, while a mod imported into the wrong sandbox is a file in
 * the wrong game's folder.
 */

/** The sandbox a drop should land in, published by whichever screen is up. */
let current: SandboxRowT | null = null

/**
 * Claim drops for a sandbox while this component is mounted.
 *
 * A module-level slot rather than a context, deliberately: the drop overlay
 * lives ABOVE the router (a drop has to work during a route change, which is
 * exactly when a context provider below it is being torn down) and there is
 * only ever one screen in front of the user. A context would need a provider
 * wrapping the router and would still have to answer "which of these three
 * mounted sandboxes is the one on screen?".
 */
export function useDropTarget(sandbox: SandboxRowT | null | undefined) {
    useEffect(() => {
        if (!sandbox) return

        current = sandbox

        return () => {
            if (current?.id === sandbox.id) current = null
        }
    }, [sandbox])
}

export default function DropImport() {
    const [dragging, setDragging] = useState(false)
    const [target, setTarget] = useState<ImportTarget | null>(null)

    const open = useCallback(
        (batch: { files: { token: string; name: string }[] }) => {
            if (batch.files.length < 1) return

            const labels: Record<string, string> = {}

            for (const file of batch.files) labels[file.token] = file.name

            setTarget({
                tokens: batch.files.map((f) => f.token),
                labels,
                sandbox: current,
                appId: current?.appId,
                origin: 'dropped',
                from: 'Dropped on the window',
            })
        },
        []
    )

    useEffect(() => {
        let live = true
        const stops: (() => void)[] = []

        void subscribe('tmc://files-drag', z.number(), (count) => {
            if (live) setDragging(count > 0)
        })
            .then((stop) => (live ? stops.push(stop) : stop()))
            .catch(() => undefined)

        void subscribe('tmc://files-dropped', DropBatchSchema, (batch) => {
            if (!live) return

            setDragging(false)
            open(batch)
        })
            .then((stop) => (live ? stops.push(stop) : stop()))
            .catch(() => undefined)

        // The one that covers a drop landing before this mounted.
        void ipc
            .importDropped()
            .then((batch) => {
                if (live) open(batch)
            })
            .catch(() => undefined)

        return () => {
            live = false

            for (const stop of stops) stop()
        }
    }, [open])

    return (
        <>
            {dragging && !target && (
                <div
                    className="pointer-events-none fixed inset-0 z-[60] flex items-center justify-center bg-black/50 p-6"
                    aria-hidden
                >
                    <div className="flex flex-col items-center gap-3 rounded-2xl border-2 border-dashed border-accent bg-surface/95 px-10 py-8 text-center">
                        <FiDownload className="size-8 text-accent" />

                        <p className="text-sm font-semibold">Drop to import</p>

                        <p className="max-w-xs text-[11px] text-muted">
                            Archives, mod files and folders. They are copied into
                            the app — nothing is moved and nothing goes into a game
                            folder until you deploy.
                        </p>
                    </div>
                </div>
            )}

            {target && (
                <ImportDialog target={target} onClose={() => setTarget(null)} />
            )}
        </>
    )
}
