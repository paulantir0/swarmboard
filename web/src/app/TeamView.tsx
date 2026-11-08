import { useEffect, useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router'
import { api } from '../api'
import { CopyButton } from '../components/CopyButton'
import { Icon } from '../components/Icon'
import { shortDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { ViewHeader } from './ViewHeader'

export function TeamView() {
  const { teamId, me, refreshMe } = useShell()
  const navigate = useNavigate()
  const detail = usePoll(() => api.team(teamId), [teamId])
  const [name, setName] = useState('')
  const [handle, setHandle] = useState(me.user.handle)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState('')
  const [busy, setBusy] = useState(false)

  const team = detail.data?.team
  const members = detail.data?.members ?? []
  const max = detail.data?.max_members ?? 5
  const isOwner = team?.owner_id === me.user.id
  const full = members.length >= max

  useEffect(() => {
    if (team) setName((n) => n || team.name)
  }, [team])

  useEffect(() => {
    if (!saved) return
    const id = window.setTimeout(() => setSaved(''), 1600)
    return () => window.clearTimeout(id)
  }, [saved])

  const run = async (fn: () => Promise<unknown>, ok?: string) => {
    setBusy(true)
    setError('')
    try {
      await fn()
      if (ok) setSaved(ok)
      await detail.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Something went wrong')
    } finally {
      setBusy(false)
    }
  }

  const rename = (e: FormEvent) => {
    e.preventDefault()
    if (!name.trim() || name.trim() === team?.name) return
    void run(async () => {
      await api.renameTeam(teamId, name.trim())
      await refreshMe()
    }, 'name')
  }

  const saveHandle = (e: FormEvent) => {
    e.preventDefault()
    const h = handle.trim().toLowerCase()
    if (!h || h === me.user.handle) return
    void run(async () => {
      const res = await api.updateMe(h)
      setHandle(res.user.handle)
      await refreshMe()
    }, 'handle')
  }

  const remove = (uid: string, label: string, self: boolean) => {
    if (!window.confirm(self ? `Leave ${team?.name}?` : `Remove ${label}?`)) return
    void run(async () => {
      await api.removeMember(teamId, uid)
      if (self) {
        await refreshMe()
        navigate('/app', { replace: true })
      }
    })
  }

  const rotate = () => {
    if (!window.confirm('Rotate the invite link? The current link stops working.')) return
    void run(() => api.rotateInvite(teamId))
  }

  return (
    <div className="view">
      <ViewHeader title="Team" />
      <div className="page">
        {!team ? (
          detail.error ? <p className="muted">{detail.error.message}</p> : null
        ) : (
          <>
            <section className="block">
              <h2 className="block-title">Name</h2>
              {isOwner ? (
                <form className="inline-form" onSubmit={rename}>
                  <input className="field" aria-label="Team name" value={name} maxLength={60} onChange={(e) => setName(e.target.value)} />
                  <button className="btn btn-sm-h" type="submit" disabled={busy || !name.trim() || name.trim() === team.name}>
                    {saved === 'name' ? 'Saved' : 'Save'}
                  </button>
                </form>
              ) : (
                <p>{team.name}</p>
              )}
            </section>

            <section className="block">
              <div className="block-head">
                <h2 className="block-title">Members</h2>
                <span className="mono muted">
                  {members.length}/{max}
                </span>
              </div>
              <ul className="member-list">
                {members.map((m) => {
                  const self = m.user_id === me.user.id
                  const canRemove = (isOwner && !self) || (self && m.role !== 'owner')
                  return (
                    <li key={m.user_id} className="member-row">
                      <span className="member-handle mono">{m.handle}</span>
                      <span className="member-email muted">{m.email}</span>
                      <span className="member-role muted">{m.role === 'owner' ? 'Owner' : self ? 'You' : ''}</span>
                      <span className="member-joined muted mono">{shortDate(m.joined_at)}</span>
                      <span className="member-act">
                        {canRemove ? (
                          <button className="btn btn-sm btn-danger" disabled={busy} onClick={() => remove(m.user_id, m.handle, self)}>
                            {self ? 'Leave' : 'Remove'}
                          </button>
                        ) : null}
                      </span>
                    </li>
                  )
                })}
              </ul>
            </section>

            <section className="block">
              <h2 className="block-title">Invite link</h2>
              {full ? (
                <p className="muted">Team is full</p>
              ) : detail.data?.invite ? (
                <div className="inline-form">
                  <input className="field mono invite-url" readOnly value={detail.data.invite.url} aria-label="Invite link" onFocus={(e) => e.target.select()} />
                  <CopyButton text={detail.data.invite.url} className="btn-sm-h" />
                  <button className="btn btn-sm-h" onClick={rotate} disabled={busy} title="Rotate">
                    <Icon name="refresh" size={14} />
                    <span className="hide-sm">Rotate</span>
                  </button>
                </div>
              ) : (
                <button className="btn" onClick={() => void run(() => api.rotateInvite(teamId))} disabled={busy}>
                  Create link
                </button>
              )}
            </section>

            <section className="block">
              <h2 className="block-title">Your handle</h2>
              <form className="inline-form" onSubmit={saveHandle}>
                <input
                  className="field mono"
                  aria-label="Handle"
                  value={handle}
                  maxLength={20}
                  onChange={(e) => setHandle(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ''))}
                />
                <button className="btn btn-sm-h" type="submit" disabled={busy || !handle.trim() || handle === me.user.handle}>
                  {saved === 'handle' ? 'Saved' : 'Save'}
                </button>
              </form>
            </section>

            {error ? <p className="error-text">{error}</p> : null}
          </>
        )}
      </div>
    </div>
  )
}
