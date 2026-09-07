/**
 * The mirrored contract's two sharp edges.
 *
 * `src/lib/api/contract.ts` is a verbatim copy of website-city's, replaced
 * wholesale by `npm run contract:sync` — so this does not test the contract's
 * design, which belongs to the server. It pins the two behaviours the app would
 * silently get wrong if a sync ever brought back a plausible-looking
 * simplification, both of which are in the gotchas list because both have
 * already cost somebody an afternoon.
 */

import { describe, expect, it } from 'vitest'

import { BrowseSortVals, QueryBool } from './contract'

describe('QueryBool', () => {
    it('reads the truthy spellings a query string actually carries', () => {
        expect(QueryBool.parse('true')).toBe(true)
        expect(QueryBool.parse('1')).toBe(true)
        expect(QueryBool.parse(true)).toBe(true)
    })

    /*
     * The whole reason `QueryBool` exists rather than `z.coerce.boolean()`.
     * `z.coerce.boolean()` is `Boolean(value)`, and `Boolean("false")` is TRUE
     * — so every "off" filter in a shared URL would arrive switched on, and the
     * user would see more results after unticking a box.
     */
    it.each(['false', '0', '', 'no', 'off'])(
        'reads %o as false, which z.coerce.boolean() would not',
        (raw) => {
            expect(QueryBool.parse(raw)).toBe(false)
        }
    )

    it('is not fooled by an arbitrary non-empty string', () => {
        expect(QueryBool.parse('yes')).toBe(false)
        expect(QueryBool.parse('maybe')).toBe(false)
    })
})

describe('BrowseSortVals', () => {
    /*
     * `players` is a deprecated alias the APP invented; the website has always
     * called it `curUsers`. It stays in the enum because an installed build is
     * not redeployed with the server and would otherwise 400 on every server
     * browse — so removing it in a "tidy up the enum" pass would break every
     * copy already out there.
     */
    it('still accepts the deprecated players alias', () => {
        expect(BrowseSortVals).toContain('players')
        expect(BrowseSortVals).toContain('curUsers')
    })
})
