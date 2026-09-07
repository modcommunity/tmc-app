import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { z } from 'zod'

/**
 * The typed boundary between React and Rust.
 *
 * Every `invoke` in the app goes through here, and every one names a zod schema
 * for what it expects back. That is not ceremony: `invoke` is typed `Promise<T>`
 * with `T` chosen by the caller, so without a parse step the TypeScript types
 * are an unchecked assertion about a Rust struct that may have moved. Parsing
 * turns a serialisation mismatch into an error naming the field.
 */

/** Rust's serialised `AppError`. */
export const IpcErrorSchema = z.object({
    code: z.enum([
        'not_authenticated',
        'auth_rejected',
        'network',
        'api',
        'invalid',
        'jail',
        'internal',
    ]),
    message: z.string(),
})

export type IpcErrorT = z.infer<typeof IpcErrorSchema>

export class IpcError extends Error {
    readonly code: IpcErrorT['code']

    constructor(error: IpcErrorT) {
        super(error.message)
        this.name = 'IpcError'
        this.code = error.code
    }
}

/**
 * Rust may also reject with a plain string — a panic caught by Tauri, or a
 * command that returned `Err(String)`. Those are always bugs, and they get a
 * generic message so an internal detail is not rendered.
 */
function toIpcError(raw: unknown): IpcError {
    const parsed = IpcErrorSchema.safeParse(raw)

    if (parsed.success) return new IpcError(parsed.data)

    console.error('[ipc] unrecognised rejection', raw)

    return new IpcError({
        code: 'internal',
        message: 'Something went wrong. Check the activity log for details.',
    })
}

export async function call<S extends z.ZodTypeAny>(
    command: string,
    schema: S,
    args?: Record<string, unknown>
): Promise<z.infer<S>> {
    let raw: unknown

    try {
        raw = await invoke(command, args)
    } catch (err) {
        throw toIpcError(err)
    }

    const parsed = schema.safeParse(raw)

    if (!parsed.success) {
        console.error(
            `[ipc] ${command} returned an unexpected shape`,
            parsed.error,
            raw
        )

        throw new IpcError({
            code: 'internal',
            message: `The app and its backend disagree about "${command}". Please report this.`,
        })
    }

    return parsed.data as z.infer<S>
}

export function isIpcError(err: unknown): err is IpcError {
    return err instanceof IpcError
}

/**
 * Something to put on screen, from anything that was thrown.
 *
 * Rust's own message when there is one — `AppError`'s `message` half is written
 * for a person and is the useful text — and a generic sentence otherwise,
 * because a raw `TypeError` in a banner tells the user nothing they can act on.
 *
 * Here rather than in each screen: four routes had grown their own identical
 * copy, and the moment one of them starts also showing `err.code` the app has
 * two different ideas of what an error looks like.
 */
export function messageOf(err: unknown): string {
    if (isIpcError(err)) return err.message

    return err instanceof Error ? err.message : 'Something went wrong.'
}

/**
 * Subscribe to a Rust-emitted event, parsed the same way a command's reply is.
 *
 * The push half of the boundary. `call` covers everything the frontend asks
 * for; this covers what Rust says on its own — currently the download queue,
 * which ticks twice a second per transfer and would be hundreds of polls a
 * second if the UI had to ask.
 *
 * Parsed through a schema for exactly the reason `call` is: `listen` returns
 * whatever the caller claims, so without this the payload type is an unchecked
 * assertion about a Rust struct that may have moved.
 *
 * A payload that fails to parse is DROPPED with a console error rather than
 * thrown: an event handler has no caller to catch for it, and one malformed
 * message must not tear down a subscription that is otherwise working.
 */
export async function subscribe<S extends z.ZodTypeAny>(
    event: string,
    schema: S,
    onEvent: (payload: z.infer<S>) => void
): Promise<() => void> {
    return listen(event, (message) => {
        const parsed = schema.safeParse(message.payload)

        if (!parsed.success) {
            console.error(`[ipc] ${event} had an unexpected shape`, parsed.error)

            return
        }

        onEvent(parsed.data as z.infer<S>)
    })
}
