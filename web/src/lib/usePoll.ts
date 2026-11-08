import { useCallback, useEffect, useRef, useState } from 'react'

export const POLL_MS = 5000

export interface Poll<T> {
  data: T | undefined
  error: Error | undefined
  loading: boolean
  reload: () => Promise<void>
  setData: (next: T | ((prev: T | undefined) => T)) => void
}

/**
 * Fetches now, then every 5s while the tab is visible, and again when it becomes visible.
 * Keeps the previous data while refreshing so views don't flicker. Stale responses are dropped.
 */
export function usePoll<T>(fetcher: () => Promise<T>, deps: unknown[], interval = POLL_MS, enabled = true): Poll<T> {
  const [data, setDataState] = useState<T | undefined>(undefined)
  const [error, setError] = useState<Error | undefined>(undefined)
  const [loading, setLoading] = useState(true)
  const seq = useRef(0)
  const fetcherRef = useRef(fetcher)
  fetcherRef.current = fetcher

  const run = useCallback(async () => {
    const id = ++seq.current
    try {
      const next = await fetcherRef.current()
      if (id !== seq.current) return
      setDataState(next)
      setError(undefined)
    } catch (e) {
      if (id !== seq.current) return
      setError(e instanceof Error ? e : new Error(String(e)))
    } finally {
      if (id === seq.current) setLoading(false)
    }
  }, [])

  useEffect(() => {
    if (!enabled) return
    setLoading(true)
    setDataState(undefined)
    setError(undefined)
    void run()
    const timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') void run()
    }, interval)
    const onVisible = () => {
      if (document.visibilityState === 'visible') void run()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      seq.current++
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', onVisible)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, enabled, interval])

  const setData = useCallback((next: T | ((prev: T | undefined) => T)) => {
    seq.current++
    setLoading(false)
    setDataState((prev) => (typeof next === 'function' ? (next as (p: T | undefined) => T)(prev) : next))
  }, [])

  return { data, error, loading, reload: run, setData }
}
