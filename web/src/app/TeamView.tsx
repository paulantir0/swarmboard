import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router'
import { api, type Member } from '../api'
import { CopyButton } from '../components/CopyButton'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { shortDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { JoinForm } from './JoinForm'
import { ViewHeader } from './ViewHeader'

const ROLE_LABEL = { owner: 'Owner', admin: 'Admin', member: '' } as const

export function TeamView() {
  const { teamId, me, team: summary, refreshMe } = useShell()
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
  const myRole = members.find((m) => m.user_id === me.user.id)?.role ?? summary.role
  const isOwner = myRole === 'owner'
  const canManage = myRole === 'owner' || myRole === 'admin'
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

  const remove = (m: Member) => {
    if (!window.confirm(`Remove ${m.handle} from ${team?.name}?`)) return
    void run(() => api.removeMember(teamId, m.user_id))
  }

  const leave = () => {
    if (!window.confirm(`Leave ${team?.name}? You'll need a new invite to come back.`)) return
    void run(async () => {
      await api.removeMember(teamId, me.user.id)
      await refreshMe()
      navigate('/app', { replace: true })
    })
  }

  const setRole = (m: Member, role: 'admin' | 'member') =>
    void run(async () => {
      await api.setRole(teamId, m.user_id, role)
    })

  const transfer = (m: Member) => {
    if (!window.confirm(`Transfer ownership of ${team?.name} to ${m.handle}? You'll become an admin.`)) return
    void run(async () => {
      await api.transferOwnership(teamId, m.user_id)
      await refreshMe()
    })
  }

  const rotate = () => {
    if (!window.confirm('Rotate the invite link? The current link stops working.')) return
    void run(() => api.rotateInvite(teamId))
  }

  return (
    <div className="view">
      <ViewHeader title="Team" />
      <Loadable ready={Boolean(team) || Boolean(detail.error)} label="Loading team">
        <div className="page">
          {!team ? (
            <p className="muted">{detail.error?.message}</p>
          ) : (
            <>
              <section className="block">
                <h2 className="block-title">Name</h2>
                {canManage ? (
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
                    const ownerMenu = isOwner && !self
                    const adminRemove = myRole === 'admin' && !self && m.role === 'member'
                    return (
                      <li key={m.user_id} className="member-row">
                        <span className="member-handle mono">{m.handle}</span>
                        <span className="member-email muted">{m.email}</span>
                        <span className="member-role">
                          {ROLE_LABEL[m.role] ? <span className={`role-badge ${m.role}`}>{ROLE_LABEL[m.role]}</span> : null}
                          {self ? <span className="muted">You</span> : null}
                        </span>
                        <span className="member-joined muted mono">{shortDate(m.joined_at)}</span>
                        <span className="member-act">
                          {ownerMenu ? (
                            <RowMenu
                              label={`Manage ${m.handle}`}
                              disabled={busy}
                              items={[
                                m.role === 'admin'
                                  ? { label: 'Remove admin', onSelect: () => setRole(m, 'member') }
                                  : { label: 'Make admin', onSelect: () => setRole(m, 'admin') },
                                { label: 'Transfer ownership', onSelect: () => transfer(m) },
                                { label: 'Remove', danger: true, onSelect: () => remove(m) },
                              ]}
                            />
                          ) : adminRemove ? (
                            <button className="btn btn-sm btn-danger" disabled={busy} onClick={() => remove(m)}>
                              Remove
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
                    {canManage ? (
                      <button className="btn btn-sm-h" onClick={rotate} disabled={busy} title="Rotate">
                        <Icon name="refresh" size={14} />
                        <span className="hide-sm">Rotate</span>
                      </button>
                    ) : null}
                  </div>
                ) : canManage ? (
                  <button className="btn" onClick={() => void run(() => api.rotateInvite(teamId))} disabled={busy}>
                    Create link
                  </button>
                ) : (
                  <p className="muted">No invite link</p>
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

              <section className="block">
                <h2 className="block-title">Join a board</h2>
                <JoinForm refreshMe={refreshMe} />
              </section>

              {!isOwner ? (
                <section className="block">
                  <button className="btn btn-danger" onClick={leave} disabled={busy}>
                    <Icon name="logOut" size={14} />
                    Leave board
                  </button>
                </section>
              ) : null}

              {error ? <p className="error-text">{error}</p> : null}
            </>
          )}
        </div>
      </Loadable>
    </div>
  )
}

/** A "…" button with a small menu of row actions. */
function RowMenu({ label, items, disabled }: { label: string; disabled?: boolean; items: { label: string; danger?: boolean; onSelect: () => void }[] }) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    document.addEventListener('mousedown', close)
    document.addEventListener('keydown', esc)
    return () => {
      document.removeEventListener('mousedown', close)
      document.removeEventListener('keydown', esc)
    }
  }, [open])

  return (
    <div className="row-menu" ref={ref}>
      <button
        className="btn btn-ghost btn-sm btn-icon"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name="more" size={14} />
      </button>
      {open ? (
        <div className="menu row-menu-list" role="menu">
          {items.map((it) => (
            <button
              key={it.label}
              role="menuitem"
              className={`menu-item ${it.danger ? 'danger' : ''}`}
              onClick={() => {
                setOpen(false)
                it.onSelect()
              }}
            >
              <span className="menu-label">{it.label}</span>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}
