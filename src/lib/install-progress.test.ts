import { describe, expect, it } from 'vitest'

import { installPercent } from './install-progress'

const d = (
    status: 'running' | 'queued' | 'done',
    total: number | null,
    done: number,
    updatedAt = 'b'
) => ({
    status,
    total,
    done,
    updatedAt,
})

describe('installPercent', () => {
    it('is null with nothing running or no size', () => {
        expect(installPercent([])).toBeNull()
        expect(
            installPercent([d('queued', 10, 0), d('running', null, 5)])
        ).toBeNull()
    })

    it('reports the most recently updated running download, capped at 100', () => {
        expect(
            installPercent([d('running', 200, 50, 'a'), d('running', 100, 40, 'c')])
        ).toBe(40)
        expect(installPercent([d('running', 10, 11)])).toBe(100)
    })
})
