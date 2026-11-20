import { describe, expect, it, vi } from 'vitest'

vi.mock('~/lib/api/client', () => ({ api: { studioPut: vi.fn(() => Promise.resolve({ ok: 1 })) } }))
vi.mock('~/lib/ipc/commands', () => ({ ipc: { avatarMePut: vi.fn(() => Promise.resolve({ saved: true })) } }))

import { api } from '~/lib/api/client'
import { IpcError } from '~/lib/ipc'
import { ipc } from '~/lib/ipc/commands'
import { statusOf, writeStudio } from './studio-data'

describe('studio writes', () => {
    it('sends each write to its own route', async () => {
        await writeStudio('wearables', { wearableId: 1, equipped: true })
        expect(api.studioPut).toHaveBeenCalledWith('/avatar/wearables', { wearableId: 1, equipped: true })

        await writeStudio('outfits', { outfits: [] })
        expect(api.studioPut).toHaveBeenLastCalledWith('/avatar/outfits', { outfits: [] })
    })

    it("sends a game's avatar through the avatar command, not the app API", async () => {
        const r = await writeStudio('app-avatar', { appId: 3 })

        expect(ipc.avatarMePut).toHaveBeenCalledWith({ appId: 3 })
        expect(r).toEqual({ ok: true, data: { saved: true } })
    })

    it('maps an IPC failure to the studio status', () => {
        expect(statusOf(new IpcError({ code: 'not_authenticated', message: 'x' }))).toBe(401)
        expect(statusOf(new IpcError({ code: 'network', message: 'x' }))).toBe(0)
        expect(statusOf(new IpcError({ code: 'api', message: 'x' }))).toBe(400)
        expect(statusOf(new Error('x'))).toBe(0)
    })
})
