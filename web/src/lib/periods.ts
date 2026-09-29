/** A half-open UTC time range `[from, to)` as RFC 3339 timestamps. */
export interface TimeRange {
  from: string
  to: string
}

/** The UTC day `YYYY-MM-DD`. */
export function dayRange(date: string): TimeRange {
  const from = new Date(`${date}T00:00:00Z`)
  const to = new Date(from)
  to.setUTCDate(to.getUTCDate() + 1)
  return { from: from.toISOString(), to: to.toISOString() }
}

/** The UTC month `YYYY-MM`. */
export function monthRange(month: string): TimeRange {
  const from = new Date(`${month}-01T00:00:00Z`)
  const to = new Date(from)
  to.setUTCMonth(to.getUTCMonth() + 1)
  return { from: from.toISOString(), to: to.toISOString() }
}

export type Granularity = 'day' | 'month' | 'year'

/** The granularity of a tier period: `YYYY-MM-DD`, `YYYY-MM` or `YYYY`. */
export function granularityOf(period: string): Granularity | undefined {
  if (/^\d{4}-\d{2}-\d{2}$/.test(period)) return 'day'
  if (/^\d{4}-\d{2}$/.test(period)) return 'month'
  if (/^\d{4}$/.test(period)) return 'year'
  return undefined
}

/** The current period of a granularity, in the given time zone. */
export function currentPeriod(granularity: Granularity, timeZone: string, now = new Date()): string {
  // `en-CA` formats dates as `YYYY-MM-DD`.
  const today = new Intl.DateTimeFormat('en-CA', { timeZone }).format(now)
  return granularity === 'day' ? today : granularity === 'month' ? today.slice(0, 7) : today.slice(0, 4)
}

/** Converts a period to another granularity, keeping as much as possible. */
export function convertPeriod(period: string, granularity: Granularity): string {
  const [year, month = '01', day = '01'] = period.split('-')
  if (granularity === 'year') return `${year}`
  if (granularity === 'month') return `${year}-${month}`
  return `${year}-${month}-${day}`
}

/** The period before or after, as the same granularity. */
export function shiftPeriod(period: string, steps: number): string {
  const granularity = granularityOf(period)
  const [year = 0, month = 1, day = 1] = period.split('-').map(Number)
  const date = new Date(Date.UTC(year, month - 1, day))
  if (granularity === 'day') date.setUTCDate(date.getUTCDate() + steps)
  else if (granularity === 'month') date.setUTCMonth(date.getUTCMonth() + steps)
  else date.setUTCFullYear(date.getUTCFullYear() + steps)
  return convertPeriod(date.toISOString().slice(0, 10), granularity ?? 'day')
}
