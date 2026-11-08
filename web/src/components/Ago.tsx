import { useEffect, useId, useSyncExternalStore } from 'react'
import { fullDate } from '../lib/time'

/*
 * Live relative times. Every <Ago>/<Until> on screen shares one timer: it ticks each second while
 * anything visible is within a minute of now, and every 15s otherwise. No per-item intervals.
 */

const SEC = 1000
const MIN = 60 * SEC
const HOUR = 60 * MIN

const watched = new Map<string, number>()
const listeners = new Set<() => void>()
let now = Date.now()
let timer = 0
let period = 0

function schedule() {
  if (!watched.size) {
    window.clearInterval(timer)
    timer = 0
    period = 0
    return
  }
  const t = Date.now()
  let fast = false
  for (const ts of watched.values()) {
    if (Math.abs(t - ts) < MIN + SEC) {
      fast = true
      break
    }
  }
  const want = fast ? SEC : 15 * SEC
  if (want === period && timer) return
  window.clearInterval(timer)
  period = want
  timer = window.setInterval(tick, want)
}

function tick() {
  now = Date.now()
  for (const fn of listeners) fn()
  schedule()
}

const store = {
  subscribe(fn: () => void) {
    listeners.add(fn)
    return () => listeners.delete(fn)
  },
  snapshot: () => now,
}

function useNow(id: string, ts: number): number {
  useEffect(() => {
    watched.set(id, ts)
    now = Date.now()
    schedule()
    return () => {
      watched.delete(id)
      schedule()
    }
  }, [id, ts])
  return useSyncExternalStore(store.subscribe, store.snapshot, store.snapshot)
}

/** Server timestamps are ms; tolerate seconds. */
const ms = (v: number) => (v < 1e12 ? v * 1000 : v)

const dateFmt = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' })
const dateYearFmt = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric' })

function startOfDay(t: number): number {
  const d = new Date(t)
  d.setHours(0, 0, 0, 0)
  return d.getTime()
}

/** "just now", "5s ago", "3m ago", "2h ago", "yesterday", "Nov 8" (with the year if not this year). */
export function formatAgo(at: number, t = Date.now()): string {
  const d = Math.max(0, t - at)
  if (d < 2 * SEC) return 'just now'
  if (d < MIN) return `${Math.floor(d / SEC)}s ago`
  if (d < HOUR) return `${Math.floor(d / MIN)}m ago`
  const today = startOfDay(t)
  if (at >= today) return `${Math.floor(d / HOUR)}h ago`
  if (at >= today - 24 * HOUR) return d < 6 * HOUR ? `${Math.floor(d / HOUR)}h ago` : 'yesterday'
  const date = new Date(at)
  return date.getFullYear() === new Date(t).getFullYear() ? dateFmt.format(date) : dateYearFmt.format(date)
}

/** "in 45s", "in 29m", "in 2h 5m"; "now" once it has passed. */
export function formatUntil(at: number, t = Date.now()): string {
  const d = at - t
  if (d <= 0) return 'now'
  if (d < MIN) return `in ${Math.ceil(d / SEC)}s`
  if (d < HOUR) return `in ${Math.round(d / MIN)}m`
  const m = Math.round(d / MIN)
  return `in ${Math.floor(m / 60)}h${m % 60 ? ` ${m % 60}m` : ''}`
}

/** Live "5s ago"; the absolute local time is in the tooltip. */
export function Ago({ ts, className = '', prefix }: { ts: number; className?: string; prefix?: string }) {
  const at = ms(ts)
  const t = useNow(useId(), at)
  return (
    <time className={`ago ${className}`} dateTime={new Date(at).toISOString()} title={fullDate(at)}>
      {prefix ? `${prefix} ` : ''}
      {formatAgo(at, Math.max(t, Date.now() - 500))}
    </time>
  )
}

/** Live "in 29m" for deadlines (lease expiry). `expired` replaces it once past. */
export function Until({ ts, className = '', prefix, expired = 'expired' }: { ts: number; className?: string; prefix?: string; expired?: string }) {
  const at = ms(ts)
  const t = useNow(useId(), at)
  const left = formatUntil(at, Math.max(t, Date.now() - 500))
  return (
    <time className={`ago ${className}`} dateTime={new Date(at).toISOString()} title={fullDate(at)}>
      {left === 'now' ? expired : `${prefix ? `${prefix} ` : ''}${left}`}
    </time>
  )
}
