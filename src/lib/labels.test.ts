/**
 * The two smallest decisions in the app, both of which have bitten.
 *
 * `appLabel` exists because a blank, selectable row in a game picker is
 * something a user can neither read nor undo. `opensExternally` exists because
 * an article rendered through the app's markdown subset loses exactly the parts
 * that carried its layout.
 */

import { describe, expect, it } from 'vitest'

import { appLabel } from './api/labels'
import { opensExternally } from './external'

describe('appLabel', () => {
    it('uses the name it was given', () => {
        expect(appLabel({ id: 1, name: 'Grand Theft Auto V' })).toBe(
            'Grand Theft Auto V'
        )
    })

    /*
     * The case that produced unreadable rows in the browse filter's game
     * picker. The site abbreviates to `nameShort` because its chrome is built
     * around one chosen game; the app shows a flat list of the whole catalogue,
     * where an unset — or `''`, which `??` happily returns — abbreviation is a
     * blank option somebody can select.
     */
    it.each(['', '   ', '\t\n'])('never renders %o as a label', (name) => {
        const label = appLabel({ id: 42, name })

        expect(label.trim()).not.toBe('')
        expect(label).toContain('42')
    })

    it('trims a name that has content, rather than padding a row', () => {
        expect(appLabel({ id: 1, name: '  Rust  ' })).toBe('Rust')
    })
})

describe('opensExternally', () => {
    /** Articles, and only articles. Everything else has a body the app's
     * markdown renderer covers completely. */
    it('sends articles to the browser', () => {
        expect(opensExternally('article')).toBe(true)
    })

    it.each(['mod', 'asset', 'server', 'collection', 'community'] as const)(
        'renders %s in-app',
        (kind) => {
            expect(opensExternally(kind)).toBe(false)
        }
    )
})
