/*
 * Pixel-grid loader, adapted from Beautiful UI's "Loading State" (https://www.beautifului.dev/#loading-state):
 * the 3x3 grid with the chevron (Drive / Dots) and perimeter (Orbit) wavefronts, the `pixel-on` and
 * `shimmer-text` keyframes, and the live elapsed timer in mono tabular figures. Ported from Tailwind to
 * our tokens (app.css, `.loader*`), timed from when loading began, and shown only after a short delay.
 *
 * MIT License
 *
 * Copyright (c) 2026 Shane Levine
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
 * associated documentation files (the "Software"), to deal in the Software without restriction,
 * including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense,
 * and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so,
 * subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all copies or substantial
 * portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT
 * LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
 * NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
 * WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
 * SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
 */
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'

/* Drive: square cells, a chevron wavefront driving right. The 650ms cycle is shorter than the sweep,
 * so two fronts are always in flight. Dots: same wavefront, round cells. Orbit: a comet lapping the
 * perimeter. Reduced motion freezes the grid to its dim state; the timer still ticks. */
const chevron = Array.from({ length: 9 }, (_, i) => {
  const r = Math.floor(i / 3)
  const c = i % 3
  return (c + Math.abs(r - 1)) * 90
})

const ORBIT_ORDER = [0, 1, 2, 5, 8, 7, 6, 3]
const orbit = Array.from({ length: 9 }, (_, i) => {
  const k = ORBIT_ORDER.indexOf(i)
  return k === -1 ? null : k * 110
})

const PATTERNS = {
  drive: { delays: chevron, dur: 650, round: false },
  dots: { delays: chevron, dur: 650, round: true },
  orbit: { delays: orbit, dur: 950, round: false },
} satisfies Record<string, { delays: (number | null)[]; dur: number; round: boolean }>

export type LoaderVariant = keyof typeof PATTERNS

export function LoaderGrid({ variant = 'drive' }: { variant?: LoaderVariant }) {
  const { delays, dur, round } = PATTERNS[variant]
  return (
    <span aria-hidden="true" className={`loader-grid ${round ? 'round' : ''}`}>
      {delays.map((delay, i) => (
        <span
          key={i}
          className={`loader-cell ${delay === null ? 'off' : ''}`}
          style={delay === null ? undefined : ({ '--d': `${delay}ms`, '--dur': `${dur}ms` } as CSSProperties)}
        />
      ))}
    </span>
  )
}

function formatElapsed(ms: number): string {
  const total = Math.max(0, ms) / 1000
  if (total < 60) return `${total.toFixed(1)}s`
  return `${Math.floor(total / 60)}m ${(total % 60).toFixed(1)}s`
}

/** Ticks every 100ms, measured from `since` (performance.now()). */
function useElapsed(since: number) {
  const [now, setNow] = useState(() => performance.now())
  useEffect(() => {
    const id = window.setInterval(() => setNow(performance.now()), 100)
    return () => window.clearInterval(id)
  }, [])
  return formatElapsed(now - since)
}

/** Grid + shimmering label + elapsed time. */
export function PixelLoader({ label = 'Loading', since, variant }: { label?: string; since?: number; variant?: LoaderVariant }) {
  const start = useRef(since ?? performance.now())
  const elapsed = useElapsed(since ?? start.current)
  return (
    <div role="status" aria-live="polite" className="loader">
      <LoaderGrid variant={variant} />
      <span className="loader-label">{label}</span>
      <span className="loader-time" aria-hidden="true">
        {elapsed}
      </span>
    </div>
  )
}

const SHOW_AFTER_MS = 150
const FADE_MS = 180

/**
 * Shows `children` once `ready`. While not ready, a centred PixelLoader appears after 150ms (so fast
 * loads never flash it); when data arrives the loader fades out as the content fades in.
 */
export function Loadable({
  ready,
  children,
  label,
  className = '',
}: {
  ready: boolean
  children: ReactNode
  label?: string
  className?: string
}) {
  const since = useRef(performance.now())
  const [loader, setLoader] = useState<'hidden' | 'shown' | 'leaving'>('hidden')
  const shownRef = useRef(false)

  useEffect(() => {
    if (ready) {
      if (!shownRef.current) return
      shownRef.current = false
      setLoader('leaving')
      const id = window.setTimeout(() => setLoader('hidden'), FADE_MS)
      return () => window.clearTimeout(id)
    }
    since.current = performance.now()
    const id = window.setTimeout(() => {
      shownRef.current = true
      setLoader('shown')
    }, SHOW_AFTER_MS)
    return () => window.clearTimeout(id)
  }, [ready])

  return (
    <div className={`loadable ${className}`}>
      {ready ? <div className="loadable-content">{children}</div> : null}
      {loader !== 'hidden' ? (
        <div className={`loadable-loader ${loader === 'leaving' || ready ? 'leaving' : ''}`}>
          <PixelLoader label={label} since={since.current} />
        </div>
      ) : null}
    </div>
  )
}

/** Full-height boot loader (before /api/me answers). */
export function BootLoader() {
  const [show, setShow] = useState(false)
  const since = useRef(performance.now())
  useEffect(() => {
    const id = window.setTimeout(() => setShow(true), SHOW_AFTER_MS)
    return () => window.clearTimeout(id)
  }, [])
  return <div className="app-loading">{show ? <PixelLoader variant="orbit" since={since.current} /> : null}</div>
}
