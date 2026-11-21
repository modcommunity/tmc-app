import { describe, expect, it } from 'vitest'

import { normalizeWebUrl } from './web-link'

const O = 'https://moddingcommunity.com'
const app = { id: 3, url: 'gmod' }

describe('normalizeWebUrl', () => {
    it('fixes legacy article links', () => {
        expect(
            normalizeWebUrl({
                kind: 'article',
                id: '51',
                url: null,
                webUrl: `${O}/blog/view/51`,
            })
        ).toBe(`${O}/blog/51`)
        expect(
            normalizeWebUrl({
                kind: 'article',
                id: '51',
                url: 'hello',
                webUrl: `${O}/blog/view/51`,
            })
        ).toBe(`${O}/blog/hello`)
    })
    it('fixes users, mods, servers, maps', () => {
        expect(
            normalizeWebUrl({
                kind: 'user',
                id: 'u1',
                url: 'bob',
                webUrl: `${O}/user/bob`,
            })
        ).toBe(`${O}/u/bob`)
        expect(
            normalizeWebUrl({ kind: 'mod', id: '7', app, webUrl: `${O}/m/7` })
        ).toBe(`${O}/gmod/m/7`)
        expect(
            normalizeWebUrl({ kind: 'server', id: '9', app, webUrl: `${O}/s/9` })
        ).toBe(`${O}/gmod/s/9`)
        expect(
            normalizeWebUrl({
                kind: 'serverMap',
                id: '2',
                url: 'de_dust',
                app,
                webUrl: `${O}/s/m/2`,
            })
        ).toBe(`${O}/gmod/servers/maps/m/de_dust`)
    })
    it('leaves correct urls alone', () => {
        const w = `${O}/blog/51-x`
        expect(normalizeWebUrl({ kind: 'article', id: '51', webUrl: w })).toBe(w)
        expect(
            normalizeWebUrl({
                kind: 'mod',
                id: '7',
                app,
                webUrl: `${O}/gmod/m/7-x`,
            })
        ).toBe(`${O}/gmod/m/7-x`)
        expect(
            normalizeWebUrl({ kind: 'x' as never, id: '1', webUrl: 'garbage' })
        ).toBe('garbage')
    })
})
