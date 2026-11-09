import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Link } from 'react-router'
import { api, type Activity, type PostStatus } from '../api'
import { Ago } from '../components/Ago'
import { Empty } from '../components/Empty'
import { Loadable } from '../components/Loader'
import { Who } from '../components/Marks'
import { Title } from '../components/Title'
import { STATUS_LABEL } from '../lib/labels'
import { POLL_MS } from '../lib/usePoll'
import { useShell } from './context'
import { useArrivals } from './motion'
import { ViewHeader } from './ViewHeader'

const KEEP = 300
const PAGE = 200

/** Latest events once, then poll for anything newer than the last id seen. */
function useActivity(teamId: string) {
  const [events, setEvents] = useState<Activity[] | null>(null)
  const [error, setError] = useState('')
  const [attempt, setAttempt] = useState(0)
  const last = useRef(0)

  useEffect(() => {
    let alive = true
    let busy = false
    last.current = 0
    setEvents(null)
    setError('')

    const pull = async (initial: boolean) => {
      if (busy) return
      busy = true
      try {
        const got: Activity[] = []
        if (initial) {
          // No cursor: the server returns the latest events, oldest first.
          got.push(...(await api.activity(teamId, undefined, KEEP)))
        } else {
          for (let i = 0; i < 5; i++) {
            const page = await api.activity(teamId, last.current, PAGE)
            if (!alive) return
            got.push(...page)
            if (page.length) last.current = Math.max(last.current, page[page.length - 1].id)
            if (page.length < PAGE) break
          }
        }
        if (!alive) return
        if (initial && got.length) last.current = Math.max(...got.map((e) => e.id))
        setError('')
        setEvents((prev) => {
          if (!got.length && prev) return prev
          const seen = new Set((prev ?? []).map((e) => e.id))
          const merged = [...(prev ?? []), ...got.filter((e) => !seen.has(e.id))]
          return merged.slice(-KEEP)
        })
      } catch (e) {
        if (alive) setError(e instanceof Error ? e.message : 'Could not load activity')
        if (alive && initial) setEvents((prev) => prev ?? [])
      } finally {
        busy = false
      }
    }

    void pull(true)
    const timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') void pull(false)
    }, POLL_MS)
    return () => {
      alive = false
      window.clearInterval(timer)
    }
  }, [teamId, attempt])

  return { events, error, retry: () => setAttempt((n) => n + 1) }
}

export function ActivityView() {
  const { teamId, base, me } = useShell()
  const { events, error, retry } = useActivity(teamId)
  const list = events ? [...events].reverse() : null

  return (
    <div className="view">
      <ViewHeader title="Activity" />
      <Loadable ready={Boolean(list) || Boolean(error)} label="Loading activity">
        {error && !list?.length ? (
          <Empty
            title="Could not load activity"
            hint={error}
            action={
              <button className="btn btn-sm" onClick={retry}>
                Retry
              </button>
            }
          />
        ) : list?.length ? (
          <ActivityList events={list} base={base} meId={me.user.id} />
        ) : (
          <Empty title="No activity yet" hint="Posts, claims, replies and pings from agents and people show up here as they happen." />
        )}
      </Loadable>
    </div>
  )
}

/** Agents reconnect often (every new session). Keep only the newest "connected" in a run where
 * that agent did nothing else, so reconnects don't bury the work. Events are newest-first. */
function collapseReconnects(events: Activity[]): Activity[] {
  const lastWasConnect = new Map<string, boolean>()
  return events.filter((ev) => {
    const key = ev.actor?.agent_id ?? ev.actor?.user_id
    if (!key) return true
    const connect = ev.kind === 'agent_joined'
    const skip = connect && lastWasConnect.get(key) === true
    lastWasConnect.set(key, connect)
    return !skip
  })
}

/** Newest-first event feed. Events that arrive after the first render slide in once. */
export function ActivityList({ events: all, base, meId }: { events: Activity[]; base: string; meId: string }) {
  const events = useMemo(() => collapseReconnects(all), [all])
  const isNew = useArrivals(events.map((e) => e.id))
  return (
    <ul className="feed">
      {events.map((ev) => (
        <li key={ev.id} className={`act ${isNew(ev.id) ? 'arrive' : ''}`}>
          <Ago ts={ev.at} className="act-time muted" />
          <span className="act-line">
            {ev.actor ? (
              <Who author={ev.actor} me={meId} />
            ) : person(ev.data?.by) ? (
              <Handle name={person(ev.data?.by)!} />
            ) : (
              <span className="muted">someone</span>
            )}{' '}
            {describe(ev, base)}
          </span>
        </li>
      ))}
    </ul>
  )
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v ? v : undefined
}

/** A person named in event data: a handle string or an author-like object. */
function person(v: unknown): string | undefined {
  if (typeof v === 'string') return v || undefined
  if (v && typeof v === 'object') {
    const o = v as { display?: unknown; handle?: unknown }
    return str(o.display) ?? str(o.handle)
  }
  return undefined
}

function statusLabel(v: unknown): string | undefined {
  const s = str(v)
  return s ? (STATUS_LABEL[s as PostStatus] ?? s).toLowerCase() : undefined
}

function describe(ev: Activity, base: string): ReactNode {
  const d = ev.data ?? {}
  const post =
    ev.post_number != null ? (
      <Link className="act-post" to={`${base}/p/${ev.post_number}`} title={ev.post_title ?? undefined}>
        <span className="mono">#{ev.post_number}</span>
        {ev.post_title ? (
          <span className="act-title">
            {' '}
            <Title text={ev.post_title} />
          </span>
        ) : null}
      </Link>
    ) : null
  const to = statusLabel(d.to ?? d.status)
  // Lease events name the scope ("server/src/auth/**", "ci", "#12"), optionally within a repo.
  const scope = str(d.scope)
  const repo = str(d.repo)
  const leased = scope ? (
    <span className="act-scope" title={repo}>
      <span className="mono">{scope}</span>
      {repo ? (
        <span className="muted">
          {' '}
          in <span className="mono">{repo}</span>
        </span>
      ) : null}
    </span>
  ) : (
    post
  )
  switch (ev.kind) {
    case 'post_created':
      return <>posted {post}</>
    case 'post_updated':
      return <>edited {post}</>
    case 'status_changed':
      return (
        <>
          moved {post}
          {to ? <span className="muted"> to {to}</span> : null}
        </>
      )
    case 'claimed': {
      const from = str(d.from)
      return (
        <>
          {from ? 'took over' : 'claimed'} {leased}
          {from ? (
            <span className="muted">
              {' '}
              from <Handle name={from} />
            </span>
          ) : null}
        </>
      )
    }
    case 'released':
      return (
        <>
          released {leased}
          {to ? <span className="muted"> as {to}</span> : null}
        </>
      )
    case 'granted':
      return <>was granted {leased}</>
    case 'expired':
      return <>timed out on {leased}</>
    case 'commented':
      return (
        <>
          replied to {post}
          <Excerpt text={str(d.excerpt)} />
        </>
      )
    case 'pinged': {
      const target = str(d.target) ?? str(d.to)
      return (
        <>
          pinged {target ? <span className="mono act-target">{target}</span> : null}
          {post ? <> about {post}</> : null}
          <Excerpt text={str(d.excerpt)} />
        </>
      )
    }
    case 'board_created': {
      const key = str(d.key) ?? str(d.board)
      return <>created topic {key ? <span className="mono">{key}</span> : null}</>
    }
    case 'member_joined':
      return <>joined the team</>
    case 'role_changed': {
      const who = person(d.user)
      const role = str(d.role)
      return (
        <>
          made {who ? <Handle name={who} /> : 'someone'} {role === 'admin' ? 'an admin' : role === 'owner' ? 'the owner' : 'a member'}
        </>
      )
    }
    case 'ownership_transferred': {
      const to = person(d.to)
      return <>transferred ownership{to ? <> to <Handle name={to} /></> : null}</>
    }
    case 'member_removed': {
      const who = person(d.user)
      const self = who && ev.actor && (who === ev.actor.handle || who === ev.actor.display)
      return self ? <>left the team</> : <>removed {who ? <Handle name={who} /> : 'a member'}</>
    }
    case 'lease_force_released': {
      const holder = str(d.holder)
      return (
        <>
          force-released {leased}
          {holder ? (
            <span className="muted">
              {' '}
              from <Handle name={holder} />
            </span>
          ) : null}
        </>
      )
    }
    case 'org_created': {
      const name = str(d.name)
      return <>created the org{name ? <> {name}</> : null}</>
    }
    case 'org_renamed': {
      // Carries only what changed: the name, the URL slug, or both.
      const name = str(d.name)
      const slug = str(d.slug)
      const url = slug ? <span className="mono">/{slug}</span> : null
      if (name && slug) return <>renamed the org to {name} at {url}</>
      if (slug) return <>moved the org to {url}</>
      return <>renamed the org{name ? <> to {name}</> : null}</>
    }
    case 'agent_joined':
      return <>connected</>
    case 'invite_sent': {
      const domain = str(d.email_domain)
      return <>invited someone{domain ? <> from <span className="mono">{domain}</span></> : null}</>
    }
    default:
      return (
        <>
          {ev.kind.replace(/_/g, ' ')} {leased}
        </>
      )
  }
}

/** A person named only by handle ("mira", "paul/api-agent"): the identity style, no link. */
function Handle({ name }: { name: string }) {
  const slash = name.indexOf('/')
  return (
    <span className="who" title={name}>
      {slash > 0 ? (
        <>
          <span className="who-handle">{name.slice(0, slash + 1)}</span>
          <span className="who-agent">{name.slice(slash + 1)}</span>
        </>
      ) : (
        <span className="who-handle-solo">{name}</span>
      )}
    </span>
  )
}

function Excerpt({ text }: { text?: string }) {
  return text ? (
    <span className="act-excerpt" title={text}>
      {text}
    </span>
  ) : null
}
