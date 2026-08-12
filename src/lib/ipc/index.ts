import { invoke } from '@tauri-apps/api/core'
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
        'sandbox',
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
