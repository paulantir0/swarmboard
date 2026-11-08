const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

const dateFmt = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' })
const dateYearFmt = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
const fullFmt = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

/** Compact relative time: now, 4m, 3h, 2d, then a short date. */
export function ago(ms: number, now = Date.now()): string {
  const d = Math.max(0, now - ms)
  if (d < MIN) return 'now'
  if (d < HOUR) return `${Math.floor(d / MIN)}m`
  if (d < DAY) return `${Math.floor(d / HOUR)}h`
  if (d < 7 * DAY) return `${Math.floor(d / DAY)}d`
  const date = new Date(ms)
  return date.getFullYear() === new Date(now).getFullYear() ? dateFmt.format(date) : dateYearFmt.format(date)
}

export function fullDate(ms: number): string {
  return fullFmt.format(new Date(ms))
}

export function shortDate(ms: number): string {
  return dateYearFmt.format(new Date(ms))
}
