import { useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { flushSync } from 'react-dom'
import { Link, NavLink, useNavigate, type LinkProps, type NavLinkProps, type NavigateOptions, type To } from 'react-router'

/*
 * Small, calm motion helpers for the dashboard.
 * - Route changes: the View Transitions API crossfades the main pane (`view-transition-name: main`)
 *   with a few pixels of rise. Without it, the pane gets a CSS enter animation keyed by path.
 * - Counts roll to new values; rows that arrive after the first load slide in once.
 * Everything is instant under prefers-reduced-motion.
 */

type VTDocument = Document & { startViewTransition?: (cb: () => void) => { finished: Promise<void> } }

export const reducedMotion = () => typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches

let transitioning = false

/** Navigate inside a view transition when the browser has one (and motion is welcome). */
export function navigateWithTransition(navigate: (to: To, opts?: NavigateOptions) => void | Promise<void>, to: To, opts?: NavigateOptions) {
  const doc = document as VTDocument
  if (!doc.startViewTransition || reducedMotion() || transitioning) {
    void navigate(to, opts)
    return
  }
  transitioning = true
  document.documentElement.dataset.vt = ''
  const t = doc.startViewTransition(() => {
    flushSync(() => {
      void navigate(to, opts)
    })
  })
  void t.finished.finally(() => {
    transitioning = false
    delete document.documentElement.dataset.vt
  })
}

const plainClick = (e: MouseEvent<HTMLAnchorElement>) =>
  e.button === 0 && !e.metaKey && !e.ctrlKey && !e.shiftKey && !e.altKey && !e.defaultPrevented && (!e.currentTarget.target || e.currentTarget.target === '_self')

/** <Link> whose plain clicks navigate inside a view transition. */
export function TLink({ onClick, to, replace, state, ...rest }: LinkProps) {
  const navigate = useNavigate()
  return (
    <Link
      {...rest}
      to={to}
      replace={replace}
      state={state}
      onClick={(e) => {
        onClick?.(e)
        if (!plainClick(e)) return
        e.preventDefault()
        navigateWithTransition(navigate, to, { replace, state })
      }}
    />
  )
}

/** <NavLink> with the same transition behaviour. */
export function TNavLink({ onClick, to, replace, state, ...rest }: NavLinkProps) {
  const navigate = useNavigate()
  return (
    <NavLink
      {...rest}
      to={to}
      replace={replace}
      state={state}
      onClick={(e) => {
        onClick?.(e)
        if (!plainClick(e)) return
        e.preventDefault()
        navigateWithTransition(navigate, to, { replace, state })
      }}
    />
  )
}

/** A number that rolls up (or down) to its new value instead of snapping. */
export function Count({ value, className = '' }: { value: number | string; className?: string }) {
  const prev = useRef(value)
  const dir = useRef<'up' | 'down' | ''>('')
  if (prev.current !== value) {
    const a = Number(prev.current)
    const b = Number(value)
    dir.current = Number.isFinite(a) && Number.isFinite(b) && b < a ? 'down' : 'up'
    prev.current = value
  }
  return (
    <span className={`count ${className}`}>
      <span key={String(value)} className={`count-v ${dir.current ? `roll-${dir.current}` : ''}`}>
        {value}
      </span>
    </span>
  )
}

/**
 * Tracks ids across polls. `isNew(id)` is true for ~700ms after an id first shows up, but never for
 * the first batch (that one arrives with the view). Pass `undefined` while loading to start over.
 */
export function useArrivals(ids: (string | number)[] | undefined): (id: string | number) => boolean {
  const seen = useRef<Map<string | number, number> | null>(null)
  if (!ids) seen.current = null
  else if (!seen.current) seen.current = new Map(ids.map((id) => [id, 0]))
  else {
    const now = performance.now()
    for (const id of ids) if (!seen.current.has(id)) seen.current.set(id, now)
  }
  const map = seen.current
  return (id) => {
    const t = map?.get(id)
    return Boolean(t) && performance.now() - (t as number) < 700
  }
}

/** Wraps the routed view. Mount it keyed by path: without a view transition in flight, it plays a CSS enter. */
export function RouteView({ children }: { children: ReactNode }) {
  // Decided once per mount, so the CSS enter never replays when a view transition ends.
  const [enter] = useState(() => !transitioning)
  return <div className={`route-view ${enter ? 'enter' : ''}`}>{children}</div>
}
