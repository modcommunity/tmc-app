import { useState } from 'react'
import { FiFlag } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import type { ContentKindT } from '~/lib/api/contract'
import { isIpcError } from '~/lib/ipc'
import { useAuth } from '~/lib/auth/provider'
import Select from '~/components/select'

/**
 * Report an item.
 *
 * The row lands in the same moderation queue as the website's own form, which
 * is the whole reason this exists: a report filed in the app that nobody sees
 * would be worse than no report button at all.
 *
 * THE FORM ASKS FOR A REASON IN WORDS
 * -----------------------------------
 * There is a type (spam or other, matching the site) and then free text.
 * Resisting a longer enum is deliberate: moderators read the body, and a list
 * of twelve categories only ever produces arguments about which bucket
 * something belongs in — and reports filed under the wrong one.
 *
 * WHAT HAPPENS ON A DUPLICATE
 * ---------------------------
 * The server answers success. Somebody reporting the same mod again a week
 * later because nothing has visibly happened should not get an error, and
 * should not put a second item in a queue a human reads one at a time.
 */

const TYPES = [
    { value: 'OTHER', label: 'Something else' },
    { value: 'SPAM', label: 'Spam or advertising' },
] as const

export default function ReportButton({
    kind,
    id,
    name,
}: {
    kind: ContentKindT
    id: string
    name: string
}) {
    const { status } = useAuth()
    const [open, setOpen] = useState(false)

    // A numeric id: every kind but `user` has one, and `user` is not reportable
    // through this route.
    const numeric = Number(id)

    if (status !== 'signedIn' || !Number.isInteger(numeric) || numeric <= 0)
        return null

    return (
        <>
            <button
                type="button"
                onClick={() => setOpen(true)}
                className="flex items-center gap-1.5 text-xs text-muted hover:text-danger"
            >
                <FiFlag className="size-3" />
                Report
            </button>

            {open && (
                <ReportDialog
                    kind={kind}
                    id={numeric}
                    name={name}
                    onClose={() => setOpen(false)}
                />
            )}
        </>
    )
}

function ReportDialog({
    kind,
    id,
    name,
    onClose,
}: {
    kind: ContentKindT
    id: number
    name: string
    onClose: () => void
}) {
    const [type, setType] = useState<'SPAM' | 'OTHER'>('OTHER')
    const [title, setTitle] = useState('')
    const [content, setContent] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [sent, setSent] = useState(false)

    const submit = async () => {
        setBusy(true)
        setError(null)

        try {
            await api.report({
                kind,
                id,
                type,
                title: title.trim(),
                content: content.trim(),
            })
            setSent(true)
        } catch (err) {
            setError(
                isIpcError(err)
                    ? err.message
                    : err instanceof Error
                      ? err.message
                      : 'The report could not be sent.'
            )
        } finally {
            setBusy(false)
        }
    }

    return (
        <div
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
            role="dialog"
            aria-modal="true"
            aria-label={`Report ${name}`}
            onClick={(e) => e.target === e.currentTarget && onClose()}
        >
            <div className="w-full max-w-md rounded-xl border border-border bg-surface p-4">
                {sent ? (
                    <>
                        <h2 className="text-sm font-semibold">Report sent</h2>
                        <p className="mt-1 text-xs text-muted">
                            A moderator will look at it. You will not get a
                            notification — reports are handled quietly so that
                            reporting something is not itself public.
                        </p>
                        <button
                            type="button"
                            onClick={onClose}
                            className="mt-3 rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground"
                        >
                            Close
                        </button>
                    </>
                ) : (
                    <form
                        className="flex flex-col gap-3"
                        onSubmit={(e) => {
                            e.preventDefault()
                            void submit()
                        }}
                    >
                        <div>
                            <h2 className="text-sm font-semibold">
                                Report “{name}”
                            </h2>
                            <p className="mt-1 text-xs text-muted">
                                Tell us what is wrong with it. Reports go to the
                                same place as ones filed on the website.
                            </p>
                        </div>

                        <div className="text-xs">
                            <span className="mb-1 block">Type</span>
                            <Select
                                label="Report type"
                                fullWidth
                                value={type}
                                onChange={setType}
                                options={TYPES.map((t) => ({
                                    value: t.value,
                                    label: t.label,
                                }))}
                            />
                        </div>

                        <label className="text-xs">
                            Summary
                            <input
                                value={title}
                                maxLength={120}
                                autoFocus
                                placeholder="Contains malware"
                                onChange={(e) => setTitle(e.target.value)}
                                className="mt-1 w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
                            />
                        </label>

                        <label className="text-xs">
                            What happened
                            <textarea
                                value={content}
                                maxLength={4000}
                                rows={5}
                                placeholder="Be specific — what you saw, and where."
                                onChange={(e) => setContent(e.target.value)}
                                className="mt-1 w-full resize-y rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
                            />
                            <span className="mt-1 block text-[0.7rem] text-muted">
                                {content.trim().length} of at least 10 characters
                            </span>
                        </label>

                        {error && (
                            <p className="selectable text-xs text-danger">
                                {error}
                            </p>
                        )}

                        <div className="flex gap-2">
                            <button
                                type="submit"
                                disabled={
                                    busy ||
                                    title.trim().length < 3 ||
                                    content.trim().length < 10
                                }
                                className="rounded-lg bg-danger px-3 py-1.5 text-sm text-white disabled:opacity-50"
                            >
                                {busy ? 'Sending…' : 'Send report'}
                            </button>
                            <button
                                type="button"
                                onClick={onClose}
                                className="rounded-lg border border-border px-3 py-1.5 text-sm"
                            >
                                Cancel
                            </button>
                        </div>
                    </form>
                )}
            </div>
        </div>
    )
}
