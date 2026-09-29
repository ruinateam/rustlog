import { describe, expect, it } from 'vitest'
import {
  convertPeriod,
  currentPeriod,
  dayRange,
  granularityOf,
  monthRange,
  shiftPeriod,
} from './periods'

describe('ranges', () => {
  it('covers one UTC day', () => {
    expect(dayRange('2026-03-01')).toEqual({
      from: '2026-03-01T00:00:00.000Z',
      to: '2026-03-02T00:00:00.000Z',
    })
  })

  it('covers one UTC month, across the year end', () => {
    expect(monthRange('2026-12')).toEqual({
      from: '2026-12-01T00:00:00.000Z',
      to: '2027-01-01T00:00:00.000Z',
    })
  })
})

describe('tier periods', () => {
  it('know their granularity', () => {
    expect(granularityOf('2026-03-01')).toBe('day')
    expect(granularityOf('2026-03')).toBe('month')
    expect(granularityOf('2026')).toBe('year')
    expect(granularityOf('26-3')).toBeUndefined()
  })

  it('take the date in the given time zone', () => {
    // 22:30 UTC on March 1st is already March 2nd in Moscow.
    const now = new Date('2026-03-01T22:30:00Z')
    expect(currentPeriod('day', 'Europe/Moscow', now)).toBe('2026-03-02')
    expect(currentPeriod('month', 'Europe/Moscow', now)).toBe('2026-03')
    expect(currentPeriod('year', 'UTC', now)).toBe('2026')
  })

  it('convert between granularities', () => {
    expect(convertPeriod('2026-03-15', 'month')).toBe('2026-03')
    expect(convertPeriod('2026-03', 'day')).toBe('2026-03-01')
    expect(convertPeriod('2026', 'month')).toBe('2026-01')
  })

  it('shift by their own granularity', () => {
    expect(shiftPeriod('2026-03-01', -1)).toBe('2026-02-28')
    expect(shiftPeriod('2026-12', 1)).toBe('2027-01')
    expect(shiftPeriod('2026', -1)).toBe('2025')
  })
})
