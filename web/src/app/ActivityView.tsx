import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Link } from 'react-router'
import { api, type Activity, type PostStatus } from '../api'
import { Loadable } from '../components/Loader'
import { Who } from '../components/Marks'
import { Title } from '../components/Title'
import { STATUS_LABEL } from '../lib/labels'
import { ago, fullDate } from '../lib/time'
import { POLL_MS } from '../lib/usePoll'
import { useShell } from './context'
import { ViewHeader } from './ViewHeader'

const KEEP = 300
const PAGE = 200

/** Latest events once, then poll for anything newer than the last id seen. */
function useActivity(teamId: string) {
  const [events, setEvents] = useState<Activity[] | null>(null)
  const [error, setError] = useState('')
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
  }, [teamId])

  return { events, error }
}

export function ActivityView() {
  const { teamId, me } = useShell()
  const { events, error } = useActivity(teamId)
  const list = events ? [...events].reverse() : null

  return (
    <div className="view">
      <ViewHeader title="Activity" />
      <Loadable ready={Boolean(list) || Boolean(error)} label="Loading activity">
      {!list ? (
        <div className="empty">{error ? <p className="muted">{error}</p> : null}</div>
      ) : list.length === 0 ? (
        <div className="empty">
          <p className="muted">{error || 'No activity'}</p>
        </div>
      ) : (
        <ul className="feed activity">
          {list.map((ev) => (
            <li key={ev.id} className="act">
              <span className="act-time mono muted" title={fullDate(ev.at)}>
                {ago(ev.at)}
              </span>
              <span className="act-line">
                {ev.actor ? <Who author={ev.actor} me={me.user.id} /> : <span className="muted">someone</span>} {describe(ev, teamId)}
              </span>
            </li>
          ))}
        </ul>
      )}
      </Loadable>
    </div>
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

function describe(ev: Activity, teamId: string): ReactNode {
  const d = ev.data ?? {}
  const post =
    ev.post_number != null ? (
      <Link className="act-post" to={`/app/${teamId}/p/${ev.post_number}`}>
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
      {repo ? <span className="muted mono"> in {repo}</span> : null}
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
          {from ? <span className="muted"> from {from}</span> : null}
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
          commented on {post}
          <Excerpt text={str(d.excerpt)} />
        </>
      )
    case 'pinged': {
      const target = str(d.target) ?? str(d.to)
      return (
        <>
          pinged {target ? <span className="mono">{target}</span> : null}
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
          made {who ? <span className="mono">{who}</span> : 'someone'} {role === 'admin' ? 'an admin' : role === 'owner' ? 'the owner' : 'a member'}
        </>
      )
    }
    case 'ownership_transferred': {
      const to = person(d.to)
      return <>transferred ownership{to ? <> to <span className="mono">{to}</span></> : null}</>
    }
    case 'member_removed': {
      const who = person(d.user)
      const self = who && ev.actor && (who === ev.actor.handle || who === ev.actor.display)
      return self ? <>left the team</> : <>removed {who ? <span className="mono">{who}</span> : 'a member'}</>
    }
    case 'lease_force_released':
      return (
        <>
          force-released {leased}
        </>
      )
    case 'agent_joined':
      return <>connected</>
    default:
      return (
        <>
          {ev.kind.replace(/_/g, ' ')} {leased}
        </>
      )
  }
}

function Excerpt({ text }: { text?: string }) {
  return text ? <span className="act-excerpt">{text}</span> : null
}
