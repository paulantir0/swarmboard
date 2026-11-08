import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { useLocation } from 'react-router'
import { network } from '../api'

const ROUTE_MIN_MS = 360
const BACKGROUND_DELAY_MS = 300
const FINISH_MS = 260

/**
 * Thin shimmer along the top edge: on every route change (at least briefly, longer while its data
 * loads) and for background refetches that take longer than 300ms. Polls that answer fast never show.
 */
export function TopProgress() {
  const inflight = useSyncExternalStore(network.subscribe, network.inflight, network.inflight)
  const location = useLocation()
  const [routeMin, setRouteMin] = useState(false)
  const [routeTail, setRouteTail] = useState(false)
  const [slow, setSlow] = useState(false)
  const [phase, setPhase] = useState<'idle' | 'running' | 'finishing'>('idle')
  const first = useRef(true)

  // Route change: run for at least ROUTE_MIN_MS, then until the new view's requests settle.
  useEffect(() => {
    if (first.current) {
      first.current = false
      return
    }
    setRouteMin(true)
    setRouteTail(true)
    const id = window.setTimeout(() => setRouteMin(false), ROUTE_MIN_MS)
    return () => window.clearTimeout(id)
  }, [location.pathname, location.search])

  useEffect(() => {
    if (!routeMin && inflight === 0) setRouteTail(false)
  }, [routeMin, inflight])

  // Background: only once a request has been in flight for a while.
  useEffect(() => {
    if (inflight === 0) {
      setSlow(false)
      return
    }
    const id = window.setTimeout(() => setSlow(true), BACKGROUND_DELAY_MS)
    return () => window.clearTimeout(id)
  }, [inflight > 0]) // eslint-disable-line react-hooks/exhaustive-deps

  const active = slow || routeMin || (routeTail && inflight > 0)

  useEffect(() => {
    if (active) {
      setPhase('running')
      return
    }
    setPhase((p) => (p === 'running' ? 'finishing' : p))
    const id = window.setTimeout(() => setPhase('idle'), FINISH_MS)
    return () => window.clearTimeout(id)
  }, [active])

  if (phase === 'idle') return null
  return <div className={`top-progress ${phase}`} role="progressbar" aria-label="Loading" aria-busy={phase === 'running'} />
}
