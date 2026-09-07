/**
 * The latency ladder, and the four states a row can be in.
 *
 * Tested because three separate components read them — the card, the table and
 * the server panel — and the whole reason they live in one place is that those
 * three disagreeing is a browser that tells you a server is fine on one screen
 * and dead on another.
 *
 * The states matter more than the colours. `TO` exists because "500ms" and "no
 * answer at all" are different facts, and a dash for both is how a server
 * browser earns a reputation for lying.
 */

import { describe, expect, it } from 'vitest'

import { LATENCY_TIERS, latencyTone } from './latency-graph'
import { latencyState } from './server-live'

describe('the ladder', () => {
    it('rises through the rungs at the documented boundaries', () => {
        // <90 green · <150 yellow · <200 orange · ≥200 red-orange.
        expect(latencyTone(0).text).toBe('text-lat-good')
        expect(latencyTone(89).text).toBe('text-lat-good')
        expect(latencyTone(90).text).toBe('text-lat-fair')
        expect(latencyTone(149).text).toBe('text-lat-fair')
        expect(latencyTone(150).text).toBe('text-lat-poor')
        expect(latencyTone(199).text).toBe('text-lat-poor')
        expect(latencyTone(200).text).toBe('text-lat-bad')
        expect(latencyTone(100_000).text).toBe('text-lat-bad')
    })

    /*
     * A failed probe gets its OWN tone rather than the slowest rung. They are
     * different facts and the row prints `TO` for the second precisely so they
     * cannot be confused — a shared colour would undo that at a glance.
     */
    it('gives no answer its own tone, distinct from the slowest rung', () => {
        expect(latencyTone(null).text).toBe('text-lat-dead')
        expect(latencyTone(undefined).text).toBe('text-lat-dead')
        expect(latencyTone(null).text).not.toBe(latencyTone(100_000).text)
    })

    it('never falls off the end of the ladder', () => {
        for (const rtt of [-1, 0, 0.5, Number.MAX_SAFE_INTEGER]) {
            expect(latencyTone(rtt).text).toBeTruthy()
            expect(latencyTone(rtt).stroke).toBeTruthy()
        }
    })

    /** The last rung has to be unbounded, or a slow enough server has no tone. */
    it('ends in a catch-all', () => {
        expect(LATENCY_TIERS[LATENCY_TIERS.length - 1]?.limit).toBe(Infinity)
    })
})

describe('the four states', () => {
    const base = {
        result: undefined,
        error: undefined,
        probeable: true,
        settled: false,
    }

    const online = { online: true, rttMs: 30 } as never
    const offline = { online: false, rttMs: 0 } as never

    it('is measured when a probe came back', () => {
        expect(latencyState({ ...base, result: online })).toBe('measured')
    })

    it('is a timeout when a probe went out and nothing came back', () => {
        expect(latencyState({ ...base, error: 'timed out' })).toBe('timeout')
        expect(latencyState({ ...base, result: offline })).toBe('timeout')
        expect(latencyState({ ...base, settled: true })).toBe('timeout')
    })

    it('is waiting before the first probe resolves', () => {
        expect(latencyState(base)).toBe('waiting')
    })

    /*
     * The combination the function exists for. A row with no address has no
     * error and no result, which is byte-for-byte what an in-flight first probe
     * looks like — so without `probeable` it would render as "…" forever, and
     * a user would sit waiting for a measurement that is never coming.
     */
    it('is none when there is nothing to probe, whatever else is set', () => {
        expect(latencyState({ ...base, probeable: false })).toBe('none')
        expect(latencyState({ ...base, probeable: false, settled: true })).toBe(
            'none'
        )
        expect(latencyState({ ...base, probeable: false, error: 'x' })).toBe('none')
        expect(latencyState({ ...base, probeable: false, result: online })).toBe(
            'none'
        )
    })

    /** An error wins over a stale result: the last thing that happened is what
     * the row is reporting. */
    it('reports a failure even when an older result is still around', () => {
        expect(latencyState({ ...base, result: online, error: 'timed out' })).toBe(
            'timeout'
        )
    })
})
