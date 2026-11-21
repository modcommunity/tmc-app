import { describe, expect, it } from 'vitest'

import {
    RingBuffer,
    computeStats,
    formatAgo,
    formatSpan,
    inRange,
    mergeHistory,
    monitorLabel,
    monitorPath,
    monitorUrl,
    nearestIndex,
    niceCeil,
    parseLayoutMode,
    parseRange,
    percentile,
    scaleLinear,
    smoothPath,
    xTicks,
    yTicks,
    type MonitorSample,
} from './monitor'

const s = (at: number, rtt: number | null): MonitorSample => ({
    at,
    rtt,
    players: null,
    maxPlayers: null,
})

describe('RingBuffer', () => {
    it('keeps everything under capacity, oldest first', () => {
        const b = new RingBuffer<number>(3)

        b.push(1)
        b.push(2)

        expect(b.toArray()).toEqual([1, 2])
        expect(b.size).toBe(2)
    })

    it('drops the oldest when full and keeps order across the wrap', () => {
        const b = new RingBuffer<number>(3)

        for (let i = 1; i <= 8; i++) b.push(i)

        expect(b.toArray()).toEqual([6, 7, 8])
        expect(b.size).toBe(3)
    })

    it('clears and refuses a bad capacity', () => {
        const b = new RingBuffer<number>(2)

        b.push(1)
        b.clear()
        b.push(9)

        expect(b.toArray()).toEqual([9])
        expect(() => new RingBuffer(0)).toThrow(RangeError)
        expect(() => new RingBuffer(1.5)).toThrow(RangeError)
    })
})

describe('mergeHistory', () => {
    it('puts seed samples in front and drops overlap', () => {
        const merged = mergeHistory([s(1, 10), s(2, 11), s(5, 12)], [s(4, 20), s(6, 21)])

        expect(merged.map((x) => x.at)).toEqual([1, 2, 4, 6])
    })

    it('keeps the whole seed when there is no live data yet', () => {
        expect(mergeHistory([s(1, 1)], [])).toHaveLength(1)
    })
})

describe('ranges', () => {
    it('filters to the window and keeps the session whole', () => {
        const samples = [s(0, 1), s(50_000, 2), s(100_000, 3)]

        expect(inRange(samples, '1m', 100_000).map((x) => x.at)).toEqual([50_000, 100_000])
        expect(inRange(samples, 'session', 100_000)).toHaveLength(3)
    })

    it('falls back to five minutes for junk', () => {
        expect(parseRange('15m')).toBe('15m')
        expect(parseRange('nope')).toBe('5m')
        expect(parseRange(null)).toBe('5m')
    })
})

describe('computeStats', () => {
    it('is empty-safe', () => {
        const st = computeStats([])

        expect(st.count).toBe(0)
        expect(st.avg).toBeNull()
        expect(st.p95).toBeNull()
        expect(st.jitter).toBeNull()
        expect(st.lossPct).toBe(0)
        expect(st.last).toBeNull()
    })

    it('computes min, avg, max, jitter and loss', () => {
        const st = computeStats([s(1, 10), s(2, 20), s(3, null), s(4, 40)])

        expect(st.count).toBe(4)
        expect(st.ok).toBe(3)
        expect(st.lost).toBe(1)
        expect(st.lossPct).toBe(25)
        expect(st.min).toBe(10)
        expect(st.max).toBe(40)
        expect(st.avg).toBeCloseTo(70 / 3)
        // |20-10| and |40-20| over answered probes only.
        expect(st.jitter).toBe(15)
        expect(st.last).toBe(40)
    })

    it('uses nearest-rank for p95', () => {
        const samples = Array.from({ length: 100 }, (_, i) => s(i, i + 1))

        expect(computeStats(samples).p95).toBe(95)
        expect(percentile([5], 95)).toBe(5)
        expect(percentile([], 50)).toBeNull()
        expect(percentile([1, 2, 3, 4], 50)).toBe(2)
    })

    it('reports total loss', () => {
        const st = computeStats([s(1, null), s(2, null)])

        expect(st.lossPct).toBe(100)
        expect(st.min).toBeNull()
    })
})

describe('window labels', () => {
    it('is one stable label per server', () => {
        expect(monitorLabel({ kind: 'server', id: 42 })).toBe('monitor-server-42')
        expect(monitorLabel({ kind: 'server', id: '42' })).toBe('monitor-server-42')
    })

    it('uses only characters a label allows', () => {
        const label = monitorLabel({ kind: 'addr', host: 'My Host.example.com', port: 27015 })

        expect(label).toMatch(/^monitor-addr-[a-zA-Z0-9_-]+$/)
    })

    it('separates addresses that sanitise alike and joins case variants', () => {
        const a = monitorLabel({ kind: 'addr', host: 'a.b', port: 1 })
        const b = monitorLabel({ kind: 'addr', host: 'a_b', port: 1 })
        const c = monitorLabel({ kind: 'addr', host: ' A.B ', port: 1 })

        expect(a).not.toBe(b)
        expect(a).toBe(c)
        expect(monitorLabel({ kind: 'addr', host: 'a.b', port: 2 })).not.toBe(a)
    })

    it('builds the hash route', () => {
        expect(monitorUrl({ kind: 'server', id: 7 })).toBe('index.html#/monitor/server/7')

        const path = monitorPath({
            kind: 'addr',
            host: '1.2.3.4',
            port: 27015,
            appId: 9,
            name: 'My & Server',
        })
        const q = new URLSearchParams(path.split('?')[1])

        expect(path.startsWith('/monitor/addr?')).toBe(true)
        expect(q.get('host')).toBe('1.2.3.4')
        expect(q.get('port')).toBe('27015')
        expect(q.get('app')).toBe('9')
        expect(q.get('name')).toBe('My & Server')
    })
})

describe('layout mode', () => {
    it('accepts the three modes and defaults to the graph', () => {
        expect(parseLayoutMode('split')).toBe('split')
        expect(parseLayoutMode('rcon')).toBe('rcon')
        expect(parseLayoutMode('x')).toBe('graph')
        expect(parseLayoutMode(null)).toBe('graph')
    })
})

describe('chart scale', () => {
    it('rounds up to a nice number', () => {
        expect(niceCeil(0)).toBe(1)
        expect(niceCeil(37)).toBe(50)
        expect(niceCeil(100)).toBe(100)
        expect(niceCeil(101)).toBe(200)
        expect(niceCeil(2.3)).toBe(2.5)
        expect(niceCeil(NaN)).toBe(1)
    })

    it('makes ticks from zero to a top above the max, never below the floor', () => {
        const { top, ticks } = yTicks(73, 4, 40)

        expect(ticks[0]).toBe(0)
        expect(top).toBeGreaterThanOrEqual(73)
        expect(ticks[ticks.length - 1]).toBe(top)
        expect(yTicks(2, 4, 40).top).toBeGreaterThanOrEqual(40)
        expect(ticks.length).toBeLessThanOrEqual(8)
    })

    it('spaces time ticks evenly', () => {
        expect(xTicks(0, 100, 4)).toEqual([0, 25, 50, 75, 100])
        expect(xTicks(5, 5, 4)).toEqual([5])
    })

    it('scales linearly and survives a flat domain', () => {
        expect(scaleLinear(5, 0, 10, 0, 100)).toBe(50)
        expect(scaleLinear(5, 10, 0, 0, 100)).toBe(50)
        expect(scaleLinear(3, 3, 3, 7, 9)).toBe(7)
    })

    it('finds the nearest sample', () => {
        expect(nearestIndex([], 3)).toBe(-1)
        expect(nearestIndex([10], 3)).toBe(0)
        expect(nearestIndex([0, 10, 20, 30], 12)).toBe(1)
        expect(nearestIndex([0, 10, 20, 30], 18)).toBe(2)
        expect(nearestIndex([0, 10, 20, 30], 99)).toBe(3)
        expect(nearestIndex([0, 10, 20, 30], -5)).toBe(0)
    })

    it('draws a smooth path that never overshoots the data', () => {
        expect(smoothPath([])).toBe('')
        expect(smoothPath([{ x: 0, y: 1 }])).toBe('M 0.00 1.00')
        expect(
            smoothPath([
                { x: 0, y: 0 },
                { x: 1, y: 1 },
            ])
        ).toBe('M 0.00 0.00 L 1.00 1.00')

        const d = smoothPath([
            { x: 0, y: 10 },
            { x: 10, y: 10 },
            { x: 20, y: 50 },
            { x: 30, y: 10 },
        ])
        const ys = [...d.matchAll(/(-?\d+\.\d+) (-?\d+\.\d+)/g)].map((m) => Number(m[2]))

        expect(d.startsWith('M 0.00 10.00 C')).toBe(true)
        expect(Math.max(...ys)).toBeLessThanOrEqual(50)
        expect(Math.min(...ys)).toBeGreaterThanOrEqual(10)
    })

    it('formats spans and ages', () => {
        expect(formatSpan(42_000)).toBe('42s')
        expect(formatSpan(65_000)).toBe('1m 05s')
        expect(formatSpan(3_720_000)).toBe('1h 02m')
        expect(formatAgo(0)).toBe('now')
        expect(formatAgo(30_000)).toBe('-30s')
        expect(formatAgo(120_000)).toBe('-2m')
        expect(formatAgo(3_600_000)).toBe('-1h')
    })
})
