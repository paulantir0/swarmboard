import { useState } from 'react'
import { api, ApiError, type Claims, type Lease } from '../api'
import { Ago, Until } from '../components/Ago'
import { Confirm } from '../components/Confirm'
import { Empty } from '../components/Empty'
import { Loadable } from '../components/Loader'
import { Icon } from '../components/Icon'
import { Who } from '../components/Marks'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { Claim } from './PostList'
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
      setError(`Could not release ${l.scope}: ${err instanceof Error ? err.message : 'the server did not respond'}. Try again.`)
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
            Claims {leases.length ? <span className="view-count">{leases.length} held</span> : null}
          </>
        }
      />
      {error ? (
        <p className="error-text claims-error" role="alert">
          {error}
        </p>
      ) : null}
      <Loadable ready={Boolean(data) || Boolean(claims.error)} label="Loading claims">
        {!data ? (
          <Empty
            title="Could not load claims"
            hint={claims.error?.message}
            action={
              <button className="btn btn-sm" onClick={() => void claims.reload()}>
                Retry
              </button>
            }
          />
        ) : leases.length === 0 && queue.length === 0 ? (
          <Empty title="Nothing claimed" hint="Agents claim files, resources and tasks before they touch them. Their leases show up here." />
        ) : (
          <div className={`claims ${canRelease ? 'can-release' : ''}`}>
            {byRepo(leases).map(([repo, list]) => (
              <section key={repo || '-'} aria-label={repo || 'Team-wide'}>
                <h2 className="claims-head">
                  <Icon name={repo ? 'folder' : 'users'} size={14} />
                  {repo ? (
                    <span className="mono claims-repo" title={repo}>
                      {repo}
                    </span>
                  ) : (
                    <span>Team-wide</span>
                  )}
                  <span className="group-count">{list.length}</span>
                </h2>
                {list.map((l) => {
                  const key = `${l.repo}:${l.scope}`
                  return (
                    <div key={key} className={`claim-row ${asking === key ? 'asking' : ''}`}>
                      <span className="cl-scope" title={l.kind ? `${l.kind}: ${l.scope}` : l.scope}>
                        <span className="mono">{l.scope}</span>
                      </span>
                      <span className="cl-who">
                        <Claim author={l.holder} meId={me.user.id} />
                      </span>
                      <span className="cl-note" title={l.note || undefined}>
                        {l.note}
                      </span>
                      <Until ts={l.expires_at} prefix="expires" className="cl-time muted" />
                      {canRelease ? (
                        <span className="cl-act">
                          <button
                            className="btn btn-quiet-danger btn-sm"
                            disabled={busy === key || asking === key}
                            onClick={() => setAsking(key)}
                          >
                            {busy === key ? 'Releasing…' : 'Release'}
                          </button>
                        </span>
                      ) : null}
                      {asking === key ? (
                        <Confirm
                          className="cl-confirm"
                          question={
                            <>
                              Release <span className="mono">{l.scope}</span> held by <Who author={l.holder} me={me.user.id} />?
                            </>
                          }
                          confirmLabel="Release"
                          danger
                          onConfirm={() => void release(l)}
                          onCancel={() => setAsking('')}
                        />
                      ) : null}
                    </div>
                  )
                })}
              </section>
            ))}
            {queue.length ? (
              <section aria-label="Queue">
                <h2 className="claims-head">
                  <Icon name="clock" size={14} />
                  <span>Queue</span>
                  <span className="group-count">{queue.length}</span>
                </h2>
                {queue.map((w) => (
                  <div key={`${w.repo}:${w.scope}:${w.position}:${w.waiter.display}`} className="claim-row">
                    <span className="cl-scope" title={`${w.scope}: position ${w.position} in line`}>
                      <span className="cl-pos">{w.position}</span>
                      <span className="mono">{w.scope}</span>
                    </span>
                    <span className="cl-who">
                      <Who author={w.waiter} me={me.user.id} />
                    </span>
                    <span className="cl-note">
                      {w.repo ? (
                        <span className="mono muted" title={w.repo}>
                          {w.repo}
                        </span>
                      ) : (
                        <span className="muted">team-wide</span>
                      )}
                    </span>
                    <Ago ts={w.created_at} prefix="queued" className="cl-time muted" />
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
