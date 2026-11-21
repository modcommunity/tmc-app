import { describe, expect, it } from 'vitest'

import { LIBRARY, isOwn } from './nav'

describe('isOwn', () => {
    it('matches the node and its sub-paths only on a segment boundary', () => {
        expect(isOwn(LIBRARY, '/library')).toBe(true)
        expect(isOwn(LIBRARY, '/library/x')).toBe(true)
        expect(isOwn(LIBRARY, '/installs/4')).toBe(true)
        expect(isOwn(LIBRARY, '/installsfoo')).toBe(false)
        expect(isOwn(LIBRARY, '/libraryx')).toBe(false)
    })
})
