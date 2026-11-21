import { describe, expect, it } from 'vitest'

import { canBeDefault, storeNameProblem, usageFraction } from '~/lib/stores/logic'

describe('storeNameProblem', () => {
    it('rejects blank and over-long names', () => {
        expect(storeNameProblem('   ')).not.toBeNull()
        expect(storeNameProblem('x'.repeat(65))).not.toBeNull()
        expect(storeNameProblem('x'.repeat(64))).toBeNull()
    })

    it('rejects a duplicate ignoring case and padding', () => {
        expect(storeNameProblem('big disk', [' Big Disk '])).not.toBeNull()
        expect(storeNameProblem('other', ['Big Disk'])).toBeNull()
    })
})

describe('usageFraction', () => {
    it('is used over used plus free', () => {
        expect(usageFraction({ usedBytes: 25, freeBytes: 75 })).toBe(0.25)
    })

    it('is null when free space is unknown or the volume is empty', () => {
        expect(usageFraction({ usedBytes: 5, freeBytes: null })).toBeNull()
        expect(usageFraction({ usedBytes: 0, freeBytes: 0 })).toBeNull()
    })
})

describe('canBeDefault', () => {
    it('only a present permanent non-default store', () => {
        expect(
            canBeDefault({ kind: 'permanent', isDefault: false, exists: true })
        ).toBe(true)
        expect(
            canBeDefault({ kind: 'temporary', isDefault: false, exists: true })
        ).toBe(false)
        expect(
            canBeDefault({ kind: 'permanent', isDefault: true, exists: true })
        ).toBe(false)
        expect(
            canBeDefault({ kind: 'permanent', isDefault: false, exists: false })
        ).toBe(false)
    })
})
