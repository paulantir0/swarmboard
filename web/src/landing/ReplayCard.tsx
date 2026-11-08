import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react'
import { Ago } from '../components/Ago'
import { KIND_LABEL } from '../lib/labels'
import { ORG, REPO, SNAPSHOT, STORY, type Step } from './story'
import { useMedia } from './useMedia'

/*
 * A replay of the story in ./story.ts, dressed as the dashboard's activity feed. Lines arrive every
 * 1.6-2.6s at the bottom and push older ones up and out; times are live. Hover pauses it, and so
 * does a hidden tab. Under prefers-reduced-motion it is a still snapshot.
 *
 * Mounted by ./AgentReplay only where there is room beside the copy and above the boat's mast.
 */

const STILL = '(prefers-reduced-motion: reduce)'
/** Rows kept in the DOM: always more than the card can show, so pruning happens off-screen. */
const KEEP = 12
/** Lines already on the card when it first appears, so it never starts empty. */
const PRIMED = 3

interface Row {
  id: number
  at: number
  step: Step
  fresh: boolean
}

function useHidden(): boolean {
  return useSyncExternalStore(
    (fn) => {
      document.addEventListener('visibilitychange', fn)
      return () => document.removeEventListener('visibilitychange', fn)
    },
    () => document.hidden,
    () => false,
  )
}

/** The usual 1.6-2.6s, uneven, stretched by the step's own pace. */
function gap(step: Step): number {
  const u = (Math.random() + Math.random()) / 2
  return (1600 + u * 1000) * (step.pace ?? 1)
}

export default function ReplayCard() {
  const still = useMedia(STILL)
  const [hover, setHover] = useState(false)
  return (
    <section
      className={`lr ${hover ? 'paused' : ''}`}
      aria-label={`Replay: agents working in ${ORG}`}
      onPointerEnter={still ? undefined : () => setHover(true)}
      onPointerLeave={still ? undefined : () => setHover(false)}
    >
      <header className="lr-head">
        <span className="lr-org">{ORG}</span>
        <span className="lr-repo">{REPO}</span>
        <span className="lr-spacer" />
        <span className="lr-live">
          <span className="lr-dot" aria-hidden="true" />
          {hover ? 'paused' : 'live'}
        </span>
      </header>
      {still ? <Snapshot /> : <Live hover={hover} />}
    </section>
  )
}

/** Reduced motion: a still slice of the story, spread over the last few minutes. */
function Snapshot() {
  const [rows] = useState<Row[]>(() => {
    const steps = STORY.slice(SNAPSHOT[0], SNAPSHOT[1])
    const now = Date.now()
    return steps.map((step, i) => ({ id: i, step, fresh: false, at: now - (steps.length - i) * 21_000 }))
  })
  return <Feed rows={rows} />
}

function Live({ hover }: { hover: boolean }) {
  const hidden = useHidden()
  const next = useRef(PRIMED)
  const seq = useRef(PRIMED)
  const left = useRef<number | null>(null)
  const [rows, setRows] = useState<Row[]>(() => {
    const now = Date.now()
    return STORY.slice(0, PRIMED).map((step, i) => ({ id: i, step, fresh: false, at: now - (PRIMED - i) * 2600 }))
  })

  // One timer for the next line. Pausing keeps what was left of the wait.
  useEffect(() => {
    if (hover || hidden) return
    let fired = false
    const step = STORY[next.current % STORY.length]
    const wait = left.current ?? gap(step)
    const t0 = performance.now()
    const timer = window.setTimeout(() => {
      fired = true
      left.current = null
      next.current++
      const id = seq.current++
      setRows((prev) => [...prev, { id, step, fresh: true, at: Date.now() }].slice(-KEEP))
    }, wait)
    return () => {
      window.clearTimeout(timer)
      if (!fired) left.current = Math.max(250, wait - (performance.now() - t0))
    }
  }, [rows, hover, hidden])

  return <Feed rows={rows} />
}

function Feed({ rows }: { rows: Row[] }) {
  const list = useRef<HTMLOListElement>(null)
  const seen = useRef(-1)

  // A new line pushes the rest up: start the list one line low, then let it glide into place.
  useLayoutEffect(() => {
    const el = list.current
    const last = rows[rows.length - 1]
    if (!el || !last || last.id === seen.current) return
    seen.current = last.id
    if (!last.fresh) return
    const li = el.lastElementChild as HTMLElement | null
    if (!li) return
    el.style.transition = 'none'
    el.style.transform = `translateY(${li.offsetHeight}px)`
    void el.offsetHeight
    el.style.transition = ''
    el.style.transform = ''
  }, [rows])

  return (
    <div className="lr-feed">
      <ol className="lr-rows" ref={list}>
        {rows.map((r) => (
          <Line key={r.id} row={r} />
        ))}
      </ol>
    </div>
  )
}

/** Head line (who, verb, object) with the time at its right, then the detail at full width. */
function Line({ row }: { row: Row }) {
  const s = row.step
  return (
    <li className={`lr-row ${row.fresh ? 'fresh' : ''}`}>
      {s.t === 'ping' ? (
        <>
          <span className="lr-head-line">
            <Who who={s.who} />
            <span className="lr-target"> → {s.to}</span>
            {s.re ? <span className="lr-num"> #{s.re}</span> : null}
          </span>
          <Ago ts={row.at} className="lr-time" />
          <span className="lr-body">{s.body}</span>
        </>
      ) : (
        <>
          <span className="lr-head-line">
            <Who who={s.who} /> {s.verb}{' '}
            {s.scope ? <span className="lr-scope">{s.scope}</span> : null}
            {s.post ? <span className="lr-num">#{s.post.n}</span> : null}
            {s.post && s.verb === 'posted' ? <span className="lr-kind">{KIND_LABEL[s.post.kind]}</span> : null}
            {s.to ? <span className="lr-muted"> to {s.to}</span> : null}
          </span>
          <Ago ts={row.at} className="lr-time" />
          {s.post && s.verb === 'posted' ? <span className="lr-title">{s.post.title}</span> : null}
          {s.note ? <span className="lr-excerpt">{s.note}</span> : null}
          {s.out ? <span className="lr-out">{s.out}</span> : null}
        </>
      )}
    </li>
  )
}

/** handle/agent: the handle muted, the agent in ink; a human is just the handle. */
function Who({ who }: { who: string }) {
  const i = who.indexOf('/')
  return i < 0 ? (
    <span className="lr-who">
      <span className="lr-solo">{who}</span>
    </span>
  ) : (
    <span className="lr-who">
      <span className="lr-handle">{who.slice(0, i + 1)}</span>
      <span className="lr-agent">{who.slice(i + 1)}</span>
    </span>
  )
}
