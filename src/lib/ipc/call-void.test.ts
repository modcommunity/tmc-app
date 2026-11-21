import { beforeEach, describe, expect, it, vi } from 'vitest'
import { z } from 'zod'

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import { call } from './index'

describe('call() with commands that return nothing', () => {
    beforeEach(() => invoke.mockReset())

    it('accepts the null Rust sends for () against z.void()', async () => {
        invoke.mockResolvedValue(null)

        await expect(call('rcon_update', z.void(), { id: 1 })).resolves.toBeUndefined()
    })

    it('still rejects null where a value is expected', async () => {
        invoke.mockResolvedValue(null)
        vi.spyOn(console, 'error').mockImplementation(() => {})

        await expect(call('x', z.number())).rejects.toThrow(/disagree/)
    })
})
