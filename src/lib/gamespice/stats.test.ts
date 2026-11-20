import { describe, expect, it } from 'vitest'

import { completion, currentStreak, dayLabel, lastPlayedLabel, longestStreak, playShare, sparkPath, statValue } from './stats'

describe('streaks', () => {
    it('counts back from today, forgiving an unplayed today', () => {
        expect(currentStreak([0, 1, 1, 1])).toBe(3)
        expect(currentStreak([1, 1, 1, 0])).toBe(3)
        expect(currentStreak([1, 0, 1, 0])).toBe(1)
        expect(currentStreak([1, 1, 0, 0])).toBe(0)
        expect(currentStreak([])).toBe(0)
    })

    it('finds the longest run', () => {
        expect(longestStreak([1, 1, 0, 1, 1, 1, 0])).toBe(3)
        expect(longestStreak([0, 0])).toBe(0)
    })
})

describe('labels', () => {
    it('labels days in UTC', () => {
        expect(dayLabel('2026-10-31', 0)).toBe('Oct 31')
        expect(dayLabel('2026-10-31', 3)).toBe('Nov 3')
    })

    it('says when a game was last played', () => {
        const now = new Date('2026-11-20T15:00:00Z')

        expect(lastPlayedLabel(null, now)).toBe('never')
        expect(lastPlayedLabel('2026-11-20', now)).toBe('today')
        expect(lastPlayedLabel('2026-11-19', now)).toBe('yesterday')
        expect(lastPlayedLabel('2026-11-15', now)).toBe('5 days ago')
        expect(lastPlayedLabel('2026-01-02', now)).toBe('Jan 2, 2026')
    })

    it('formats stat values', () => {
        expect(statValue(12345)).toBe('12,345')
        expect(statValue(1.23456)).toBe('1.23')
    })
})

describe('charts', () => {
    it('computes completion', () => {
        expect(completion(null)).toBe(0)
        expect(completion({ unlocked: 1, total: 0 })).toBe(0)
        expect(completion({ unlocked: 1, total: 3 })).toBe(33)
        expect(completion({ unlocked: 9, total: 3 })).toBe(100)
    })

    it('draws a sparkline inside its box', () => {
        expect(sparkPath([], 10, 10)).toEqual({ line: '', area: '' })

        const { line, area } = sparkPath([0, 2, 1], 100, 20, 0)

        expect(line).toBe('M0.0 20.0 L50.0 0.0 L100.0 10.0')
        expect(area.endsWith('L100.0 20 L0 20 Z')).toBe(true)
        // An all-zero series is a flat baseline, not a divide by zero.
        expect(sparkPath([0, 0], 10, 10).line).toBe('M0.0 10.0 L10.0 10.0')
    })

    it('shares days by game, folding the tail into other', () => {
        const g = (appId: number, daysPlayed: number) => ({
            appId,
            daysPlayed,
            firstPlayed: null,
            lastPlayed: null,
            recent: [],
            achievements: null,
        })
        const s = playShare([g(1, 2), g(2, 6), g(3, 1), g(4, 1)], 2)

        expect(s.map((x) => x.appId)).toEqual([2, 1, null])
        expect(s.map((x) => x.days)).toEqual([6, 2, 2])
        expect(s.reduce((n, x) => n + x.share, 0)).toBeCloseTo(1)
        expect(playShare([])).toEqual([])
    })
})
