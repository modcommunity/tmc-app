/**
 * The browse URL's encoding.
 *
 * Worth testing precisely because it has been wrong before, in ways that looked
 * like something else: a filter that silently stopped applying, a listing that
 * came back empty, a second page that repeated the first. A query string
 * carries no types, so every one of those was a string being read as the wrong
 * thing — and none of them announced itself.
 */

import { describe, expect, it } from 'vitest'

import { encodeIdList, flag, idList, num, oneOf, text } from './query-params'

const q = (search: string) => new URLSearchParams(search)

describe('flag', () => {
    it('is true only for a literal 1', () => {
        expect(flag(q('empty=1'), 'empty')).toBe(true)
    })

    /*
     * The trap the contract has its own note about: `z.coerce.boolean()` is
     * `Boolean(value)`, so the STRING "false" is true. Anything here that read
     * a flag as "present" or coerced it would make `?empty=0` — which plainly
     * means no to whoever typed it — mean yes.
     */
    it.each(['empty=0', 'empty=false', 'empty=', 'empty=true', 'empty=yes'])(
        'is not true for ?%s',
        (search) => {
            expect(flag(q(search), 'empty')).toBeUndefined()
        }
    )

    it('is undefined when absent', () => {
        expect(flag(q(''), 'empty')).toBeUndefined()
    })
})

describe('num', () => {
    it('reads a non-negative number', () => {
        expect(num(q('min=0'), 'min')).toBe(0)
        expect(num(q('min=42'), 'min')).toBe(42)
        expect(num(q('min=1.5'), 'min')).toBe(1.5)
    })

    /*
     * A dropped filter, never a NaN. The contract rejects NaN, and a rejected
     * request empties the whole listing — so one bad value in a shared link
     * would look like "this game has no mods" rather than like a bad link.
     */
    it.each(['min=abc', 'min=-1', 'min=', 'min=%20', 'min=Infinity', 'min=NaN'])(
        'drops the filter for ?%s',
        (search) => {
            expect(num(q(search), 'min')).toBeUndefined()
        }
    )

    it('is undefined when absent', () => {
        expect(num(q(''), 'min')).toBeUndefined()
    })
})

describe('idList', () => {
    it('reads a comma list', () => {
        expect(idList(q('app=1,2,3'), 'app')).toEqual([1, 2, 3])
        expect(idList(q('app=7'), 'app')).toEqual([7])
    })

    it('tolerates spaces around the commas', () => {
        expect(idList(q('app=1, 2 ,3'), 'app')).toEqual([1, 2, 3])
    })

    /** Half a filter from a stale link is closer to what the user meant than
     * none of it, and every id that survives is a real one. */
    it('drops bad entries individually', () => {
        expect(idList(q('app=1,abc,3'), 'app')).toEqual([1, 3])
        expect(idList(q('app=0,-2,1'), 'app')).toEqual([1])
        expect(idList(q('app=1.5,2'), 'app')).toEqual([2])
    })

    /*
     * `undefined`, not `[]`. An empty array is a filter that matches nothing,
     * which is the exact opposite of "no filter" — the listing would come back
     * empty and read as a catalogue with nothing in it.
     */
    it.each(['app=', 'app=abc', 'app=0', 'app=,,,'])(
        'is undefined rather than empty for ?%s',
        (search) => {
            expect(idList(q(search), 'app')).toBeUndefined()
        }
    )
})

describe('text', () => {
    /*
     * The one that broke `?search=2024` and `?search=true`: a decoder that
     * guesses a type from the SHAPE of a string turns a perfectly good search
     * term into a number or a boolean. Values arrive as strings and stay
     * strings; the contract coerces per field, where it knows what the field
     * is.
     */
    it.each(['2024', 'true', 'false', '0', 'null', '-1'])(
        'keeps %s as a string',
        (value) => {
            expect(text(q(`search=${value}`), 'search')).toBe(value)
        }
    )

    it('treats empty as absent', () => {
        expect(text(q('search='), 'search')).toBeUndefined()
        expect(text(q(''), 'search')).toBeUndefined()
    })
})

describe('oneOf', () => {
    const OSES = ['WINDOWS', 'LINUX', 'MAC'] as const

    it('accepts a member', () => {
        expect(oneOf(q('os=LINUX'), 'os', OSES)).toBe('LINUX')
    })

    /** A 400 empties the listing, and the user cannot tell that apart from
     * "no results". */
    it.each(['os=linux', 'os=BSD', 'os='])('drops ?%s', (search) => {
        expect(oneOf(q(search), 'os', OSES)).toBeUndefined()
    })
})

describe('round trip', () => {
    it('survives encode → decode', () => {
        const encoded = encodeIdList([3, 1, 2])

        expect(encoded).toBe('3,1,2')
        expect(idList(q(`app=${encoded}`), 'app')).toEqual([3, 1, 2])
    })

    /** Nothing selected clears the parameter rather than writing an empty one,
     * which `idList` would read as no filter anyway — but a `?app=` in a shared
     * URL is noise that invites somebody to "fix" it. */
    it('writes null for an empty selection', () => {
        expect(encodeIdList([])).toBeNull()
        expect(encodeIdList([0, -1])).toBeNull()
    })
})
