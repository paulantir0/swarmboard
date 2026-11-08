import { useEffect, useRef, useState } from 'react'
import { Link } from 'react-router'
import { api } from '../api'
import { Who } from '../components/Marks'
import { ago, fullDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { PingForm, replyTarget } from './PingForm'
import { ViewHeader } from './ViewHeader'

export function InboxView() {
  const { teamId, me, refreshUnread } = useShell()
  const inbox = usePoll(() => api.inbox(teamId, true), [teamId])
  const [composing, setComposing] = useState(false)
  const [replyTo, setReplyTo] = useState<string | null>(null)
  const marked = useRef(new Set<string>())

  // Viewing the inbox marks it read; unread pings keep their marker until the next refresh.
  useEffect(() => {
    const fresh = (inbox.data ?? []).filter((p) => !p.read_at && !marked.current.has(p.id))
    if (fresh.length === 0) return
    for (const p of fresh) marked.current.add(p.id)
    api
      .markInboxRead(teamId)
      .then(() => refreshUnread())
      .catch(() => {})
  }, [inbox.data, teamId, refreshUnread])

  const pings = inbox.data ?? []

  return (
    <div className="view">
      <ViewHeader title="Inbox">
        <button className="btn btn-primary" onClick={() => setComposing((c) => !c)} aria-expanded={composing}>
          Send ping
        </button>
      </ViewHeader>
      {composing ? (
        <div className="panel">
          <PingForm autoFocus />
        </div>
      ) : null}
      {inbox.loading && !inbox.data ? (
        <div className="empty" />
      ) : pings.length === 0 ? (
        <div className="empty">
          <p className="muted">No pings</p>
        </div>
      ) : (
        <ul className="feed">
          {pings.map((p) => (
            <li key={p.id} className={`ping ${p.read_at ? '' : 'unread'}`}>
              <div className="ping-head">
                <Who author={p.from} me={me.user.id} />
                <span className="ping-target mono muted">→ {p.target}</span>
                {p.post_number ? (
                  <Link className="mono" to={`/app/${teamId}/p/${p.post_number}`}>
                    #{p.post_number}
                  </Link>
                ) : null}
                <span className="spacer" />
                <span className="mono muted" title={fullDate(p.created_at)}>
                  {ago(p.created_at)}
                </span>
              </div>
              <p className="ping-body">{p.body}</p>
              <div className="ping-actions">
                <button className="btn btn-ghost btn-sm" onClick={() => setReplyTo(replyTo === p.id ? null : p.id)}>
                  Reply
                </button>
              </div>
              {replyTo === p.id ? <PingForm initialTo={replyTarget(p.from)} postNumber={p.post_number ?? undefined} autoFocus onDone={() => setReplyTo(null)} /> : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
