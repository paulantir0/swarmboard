import { Fragment, useState } from 'react'
import { Link } from 'react-router'
import { api } from '../api'
import { Loadable } from '../components/Loader'
import { ago, fullDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { PingForm } from './PingForm'
import { ViewHeader } from './ViewHeader'

export function Agents() {
  const { teamId } = useShell()
  const agents = usePoll(() => api.agents(teamId), [teamId])
  const [pinging, setPinging] = useState<string | null>(null)

  const list = [...(agents.data ?? [])].sort((a, b) => Number(b.active) - Number(a.active) || b.last_seen_at - a.last_seen_at)
  const active = list.filter((a) => a.active).length

  return (
    <div className="view">
      <ViewHeader
        title={
          <>
            Agents <span className="view-count mono">{active ? `${active} active` : ''}</span>
          </>
        }
      />
      <Loadable ready={Boolean(agents.data) || Boolean(agents.error)} label="Loading agents">
      {agents.error && !agents.data ? (
        <div className="empty">
          <p className="muted">{agents.error.message}</p>
        </div>
      ) : list.length === 0 ? (
        <div className="empty">
          <p className="muted">No agents</p>
          <Link className="btn" to={`/app/${teamId}/connect`}>
            Connect an agent
          </Link>
        </div>
      ) : (
        <div className="table agents-table" role="table">
          <div className="thead" role="row">
            <span role="columnheader">Agent</span>
            <span role="columnheader">Working on</span>
            <span role="columnheader">Claimed</span>
            <span role="columnheader" className="a-seen-h">Seen</span>
            <span role="columnheader" className="visually-hidden">
              Ping
            </span>
          </div>
          {list.map((a) => (
            <Fragment key={a.id}>
              <div className={`trow ${a.active ? '' : 'inactive'}`} role="row">
                <span className="a-who" role="cell">
                  <span className={`live-dot ${a.active ? 'on' : ''}`} title={a.active ? 'Active' : 'Inactive'} />
                  <span className="who">
                    <span className="who-handle">{a.owner_handle}/</span>
                    <span className="who-agent">{a.name}</span>
                  </span>
                  {a.client ? <span className="a-client mono">{a.client}</span> : null}
                </span>
                <span className="a-working" role="cell">
                  {a.working_on || <span className="muted">—</span>}
                </span>
                <span className="a-claimed mono" role="cell">
                  {a.claimed.length
                    ? a.claimed.map((n) => (
                        <Link key={n} to={`/app/${teamId}/p/${n}`}>
                          #{n}
                        </Link>
                      ))
                    : null}
                </span>
                <span className="a-seen mono muted" role="cell" title={fullDate(a.last_seen_at)}>
                  {ago(a.last_seen_at)}
                </span>
                <span className="a-act" role="cell">
                  <button className="btn btn-sm" onClick={() => setPinging(pinging === a.id ? null : a.id)} aria-expanded={pinging === a.id}>
                    Ping
                  </button>
                </span>
              </div>
              {pinging === a.id ? (
                <div className="trow-extra">
                  <PingForm initialTo={a.display} autoFocus />
                </div>
              ) : null}
            </Fragment>
          ))}
        </div>
      )}
      </Loadable>
    </div>
  )
}
