import { useState } from 'react'
import { api, ApiError, type Claims, type Lease } from '../api'
import { Ago, Until } from '../components/Ago'
import { Confirm } from '../components/Confirm'
import { Loadable } from '../components/Loader'
import { Who } from '../components/Marks'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { ViewHeader } from './ViewHeader'

const EMPTY: Claims = { leases: [], queue: [] }


function byRepo(leases: Lease[]): [string, Lease[]][] {
  const groups = new Map<string, Lease[]>()
  for (const l of leases) {
    const list = groups.get(l.repo) ?? []
    list.push(l)
    groups.set(l.repo, list)
  }
  for (const list of groups.values()) list.sort((a, b) => a.scope.localeCompare(b.scope))
  // Repos alphabetically, team-wide ("") last.
  return [...groups.entries()].sort(([a], [b]) => (a === '' ? 1 : b === '' ? -1 : a.localeCompare(b)))
}

export function ClaimsView() {
  const { teamId, me, team } = useShell()
  const claims = usePoll(() => api.claims(teamId), [teamId])
  const canRelease = team.role === 'owner' || team.role === 'admin'
  const [busy, setBusy] = useState('')
  const [asking, setAsking] = useState('')
  const [error, setError] = useState('')

  const release = async (l: Lease) => {
    const key = `${l.repo}:${l.scope}`
    setAsking('')
    setBusy(key)
    setError('')
    try {
      await api.releaseClaim(teamId, l.scope, l.repo)
      await claims.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not release')
    } finally {
      setBusy('')
    }
  }
  // The endpoint may not exist on older servers: treat a 404 as "nothing held".
  const missing = claims.error instanceof ApiError && claims.error.status === 404
  const data = claims.data ?? (missing ? EMPTY : undefined)
  const leases = data?.leases ?? []
  const queue = [...(data?.queue ?? [])].sort((a, b) => a.scope.localeCompare(b.scope) || a.position - b.position)

  return (
    <div className="view">
      <ViewHeader
        title={
          <>
            Claims <span className="view-count mono">{leases.length ? `${leases.length} held` : ''}</span>
          </>
        }
      />
      {error ? <p className="error-text claims-error">{error}</p> : null}
      <Loadable ready={Boolean(data) || Boolean(claims.error)} label="Loading claims">
      {!data ? (
        <div className="empty">{claims.error ? <p className="muted">{claims.error.message}</p> : null}</div>
      ) : leases.length === 0 && queue.length === 0 ? (
        <div className="empty">
          <p className="muted">No claims</p>
        </div>
      ) : (
        <div className={`claims ${canRelease ? 'can-release' : ''}`}>
          {byRepo(leases).map(([repo, list]) => (
            <section key={repo || '-'} aria-label={repo || 'Team-wide'}>
              <h2 className="claims-head">
                <span className={repo ? 'mono' : ''}>{repo || 'Team-wide'}</span>
                <span className="group-count">{list.length}</span>
              </h2>
              {list.map((l) => (
                <div key={`${l.repo}:${l.scope}`} className={`claim-row ${asking === `${l.repo}:${l.scope}` ? 'asking' : ''}`}>
                  <span className="cl-scope mono" title={l.kind ? `${l.kind}: ${l.scope}` : l.scope}>
                    {l.scope}
                  </span>
                  <span className="cl-who">
                    <Who author={l.holder} me={me.user.id} />
                  </span>
                  <span className="cl-note">{l.note || <span className="muted">—</span>}</span>
                  <span className="cl-time mono muted">
                    <Until ts={l.expires_at} prefix="expires" />
                  </span>
                  {canRelease ? (
                    <span className="cl-act">
                      <button
                        className="btn btn-sm"
                        disabled={busy === `${l.repo}:${l.scope}` || asking === `${l.repo}:${l.scope}`}
                        onClick={() => setAsking(`${l.repo}:${l.scope}`)}
                      >
                        Release
                      </button>
                    </span>
                  ) : null}
                  {asking === `${l.repo}:${l.scope}` ? (
                    <Confirm
                      className="cl-confirm"
                      question={
                        <>
                          Release <span className="mono">{l.scope}</span> held by {l.holder.display}?
                        </>
                      }
                      confirmLabel="Release"
                      danger
                      onConfirm={() => void release(l)}
                      onCancel={() => setAsking('')}
                    />
                  ) : null}
                </div>
              ))}
            </section>
          ))}
          {queue.length ? (
            <section aria-label="Queue">
              <h2 className="claims-head">
                <span>Queue</span>
                <span className="group-count">{queue.length}</span>
              </h2>
              {queue.map((w) => (
                <div key={`${w.repo}:${w.scope}:${w.position}:${w.waiter.display}`} className="claim-row">
                  <span className="cl-scope mono" title={w.repo || 'Team-wide'}>
                    <span className="cl-pos">{w.position}</span>
                    {w.scope}
                  </span>
                  <span className="cl-who">
                    <Who author={w.waiter} me={me.user.id} />
                  </span>
                  <span className="cl-note mono muted">{w.repo || 'team-wide'}</span>
                  <span className="cl-time mono muted">
                    <Ago ts={w.created_at} prefix="waiting" />
                  </span>
                  {canRelease ? <span className="cl-act" /> : null}
                </div>
              ))}
            </section>
          ) : null}
        </div>
      )}
      </Loadable>
    </div>
  )
}
