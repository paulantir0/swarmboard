import { useEffect, useState } from 'react'
import { Link } from 'react-router'
import { api } from '../api'
import { Ago } from '../components/Ago'
import { Empty } from '../components/Empty'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { Markdown } from '../components/Markdown'
import { Who } from '../components/Marks'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { PingForm, replyTarget } from './PingForm'
import { ViewHeader } from './ViewHeader'

export function InboxView() {
  const { teamId, base, me, refreshUnread } = useShell()
  const inbox = usePoll(() => api.inbox(teamId, true), [teamId])
  const [composing, setComposing] = useState(false)
  const [replyTo, setReplyTo] = useState<string | null>(null)
  const [sent, setSent] = useState<{ id: string; summary: string } | null>(null)
  // Pings that were unread when they arrived on screen keep their marker for the whole visit.
  const [unread, setUnread] = useState<ReadonlySet<string>>(() => new Set())

  // Viewing the inbox marks it read.
  useEffect(() => {
    const fresh = (inbox.data ?? []).filter((p) => !p.read_at && !unread.has(p.id))
    if (fresh.length === 0) return
    setUnread((prev) => new Set([...prev, ...fresh.map((p) => p.id)]))
    api
      .markInboxRead(teamId)
      .then(() => refreshUnread())
      .catch(() => {})
  }, [inbox.data, teamId, refreshUnread, unread])

  const pings = inbox.data ?? []

  return (
    <div className="view">
      <ViewHeader title="Inbox">
        <button className="btn btn-sm" onClick={() => setComposing((c) => !c)} aria-expanded={composing} aria-label="Send ping">
          <Icon name="send" size={14} />
          <span className="hide-sm">Send ping</span>
        </button>
      </ViewHeader>
      {composing ? (
        <div className="panel">
          <PingForm autoFocus />
        </div>
      ) : null}
      <Loadable ready={Boolean(inbox.data) || Boolean(inbox.error)} label="Loading inbox">
        {inbox.error && !inbox.data ? (
          <Empty
            title="Could not load the inbox"
            hint={inbox.error.message}
            action={
              <button className="btn btn-sm" onClick={() => void inbox.reload()}>
                Retry
              </button>
            }
          />
        ) : pings.length === 0 ? (
          <Empty title="No pings" hint="When an agent or teammate pings you, it lands here." />
        ) : (
          <ul className="feed">
            {pings.map((p) => {
              const isUnread = !p.read_at || unread.has(p.id)
              return (
                <li key={p.id} className={`ping ${isUnread ? 'unread' : ''}`}>
                  <span className="ping-dot" />
                  <div className="ping-head">
                    {isUnread ? <span className="visually-hidden">Unread: </span> : null}
                    <Who author={p.from} me={me.user.id} />
                    <span className="ping-target">
                      <span aria-hidden="true">→ </span>
                      <span className="visually-hidden">to </span>
                      <span className="mono">{p.target}</span>
                    </span>
                    {p.post_number ? (
                      <Link className="ping-post-link mono" to={`${base}/p/${p.post_number}`}>
                        #{p.post_number}
                      </Link>
                    ) : null}
                    <Ago ts={p.created_at} className="ping-time muted" />
                  </div>
                  <Markdown source={p.body} className="ping-body" />
                  <div className="ping-actions">
                    <button
                      className="btn btn-ghost btn-sm"
                      onClick={() => {
                        setReplyTo(replyTo === p.id ? null : p.id)
                        setSent(null)
                      }}
                      aria-expanded={replyTo === p.id}
                    >
                      Reply
                    </button>
                    <span className="ping-sent muted" role="status">
                      {sent?.id === p.id ? sent.summary : ''}
                    </span>
                  </div>
                  {replyTo === p.id ? (
                    <PingForm
                      initialTo={replyTarget(p.from)}
                      postNumber={p.post_number ?? undefined}
                      autoFocus
                      onDone={(summary) => {
                        setReplyTo(null)
                        setSent({ id: p.id, summary })
                      }}
                    />
                  ) : null}
                </li>
              )
            })}
          </ul>
        )}
      </Loadable>
    </div>
  )
}
