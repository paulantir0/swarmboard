import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router'
import { api, type Member } from '../api'
import { Confirm, ConfirmButton } from '../components/Confirm'
import { CopyButton } from '../components/CopyButton'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { shortDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { JoinForm } from './JoinForm'
import { slugify } from './NewOrg'
import { HOST } from './Shell'
import { ViewHeader } from './ViewHeader'

const ROLE_LABEL = { owner: 'Owner', admin: 'Admin', member: '' } as const

export function TeamView() {
  const { teamId, base, me, team: summary, refreshMe } = useShell()
  const [slug, setSlug] = useState(summary.slug)
  const navigate = useNavigate()
  const detail = usePoll(() => api.team(teamId), [teamId])
  const [name, setName] = useState('')
  const [handle, setHandle] = useState(me.user.handle)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState('')
  const [busy, setBusy] = useState(false)
  const [pending, setPending] = useState<{ uid: string; kind: 'remove' | 'transfer' } | null>(null)

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

  const saveSlug = (e: FormEvent) => {
    e.preventDefault()
    const next = slugify(slug)
    if (!next || next === team?.slug) return
    void run(async () => {
      const res = await api.setSlug(teamId, next)
      await refreshMe()
      navigate(`/${res.team.slug}/team`, { replace: true })
    }, 'slug')
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

  const remove = (m: Member) => run(() => api.removeMember(teamId, m.user_id))

  const leave = () =>
    run(async () => {
      await api.removeMember(teamId, me.user.id)
      await refreshMe()
      navigate('/app', { replace: true })
    })

  const setRole = (m: Member, role: 'admin' | 'member') =>
    void run(async () => {
      await api.setRole(teamId, m.user_id, role)
    })

  const transfer = (m: Member) =>
    run(async () => {
      await api.transferOwnership(teamId, m.user_id)
      await refreshMe()
    })

  const rotate = () => run(() => api.rotateInvite(teamId))

  const confirmPending = async (m: Member) => {
    const kind = pending?.kind
    setPending(null)
    if (kind === 'remove') await remove(m)
    else if (kind === 'transfer') await transfer(m)
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
                <h2 className="block-title">Org name</h2>
                {canManage ? (
                  <form className="inline-form" onSubmit={rename}>
                    <input className="field" aria-label="Team name" value={name} maxLength={60} onChange={(e) => setName(e.target.value)} />
                    <button className="btn" type="submit" disabled={busy || !name.trim() || name.trim() === team.name}>
                      {saved === 'name' ? 'Saved' : 'Save'}
                    </button>
                  </form>
                ) : (
                  <p>{team.name}</p>
                )}
              </section>

              <section className="block">
                <h2 className="block-title">URL</h2>
                {isOwner ? (
                  <form className="inline-form" onSubmit={saveSlug}>
                    <span className="slug-input">
                      <span className="slug-host mono">{HOST}/</span>
                      <input
                        className="field mono"
                        aria-label="URL slug"
                        value={slug}
                        maxLength={40}
                        spellCheck={false}
                        autoComplete="off"
                        onChange={(e) => setSlug(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '-'))}
                      />
                    </span>
                    <button className="btn" type="submit" disabled={busy || !slugify(slug) || slugify(slug) === team.slug}>
                      {saved === 'slug' ? 'Saved' : 'Save'}
                    </button>
                  </form>
                ) : (
                  <div className="inline-form">
                    <code className="token-value">
                      {HOST}/{team.slug}
                    </code>
                    <CopyButton text={`${window.location.origin}/${team.slug}`} size="md" />
                  </div>
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
                    const asking = pending?.uid === m.user_id ? pending.kind : null
                    return (
                      <li key={m.user_id} className={`member-row ${asking ? 'asking' : ''}`}>
                        <Link className="member-handle mono" to={`${base}/members/${m.user_id}`}>
                          {m.handle}
                        </Link>
                        <span className="member-email muted">{m.email}</span>
                        <span className="member-role">
                          {ROLE_LABEL[m.role] ? <span className={`role-badge ${m.role}`}>{ROLE_LABEL[m.role]}</span> : null}
                          {self ? <span className="muted">you</span> : null}
                        </span>
                        <span className="member-joined muted mono">{shortDate(m.joined_at)}</span>
                        <span className="member-act">
                          {asking ? (
                            <Confirm
                              className="member-confirm"
                              question={asking === 'remove' ? `Remove ${m.handle}?` : `Make ${m.handle} the owner? You become an admin.`}
                              confirmLabel={asking === 'remove' ? 'Remove' : 'Transfer'}
                              danger
                              busy={busy}
                              onConfirm={() => void confirmPending(m)}
                              onCancel={() => setPending(null)}
                            />
                          ) : ownerMenu ? (
                            <RowMenu
                              label={`Manage ${m.handle}`}
                              disabled={busy}
                              items={[
                                m.role === 'admin'
                                  ? { label: 'Remove admin', onSelect: () => setRole(m, 'member') }
                                  : { label: 'Make admin', onSelect: () => setRole(m, 'admin') },
                                { label: 'Transfer ownership', onSelect: () => setPending({ uid: m.user_id, kind: 'transfer' }) },
                                { label: 'Remove', danger: true, onSelect: () => setPending({ uid: m.user_id, kind: 'remove' }) },
                              ]}
                            />
                          ) : adminRemove ? (
                            <button className="btn btn-xs btn-danger" disabled={busy} onClick={() => setPending({ uid: m.user_id, kind: 'remove' })}>
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
                <h2 className="block-title">Invite</h2>
                {full ? (
                  <p className="muted">The org is full ({max} people)</p>
                ) : (
                  <div className="invite-rows">
                    <div className="invite-row">
                      <span className="invite-label">Link</span>
                      {detail.data?.invite ? (
                        <div className="inline-form">
                          <input className="field mono invite-url" readOnly value={detail.data.invite.url} aria-label="Invite link" onFocus={(e) => e.target.select()} />
                          <CopyButton text={detail.data.invite.url} size="md" />
                          {canManage ? (
                            <ConfirmButton
                              className="btn"
                              disabled={busy}
                              title="Rotate"
                              question="New link? The old one stops working."
                              confirmLabel="Rotate"
                              onConfirm={rotate}
                            >
                              <Icon name="refresh" size={14} />
                              <span className="hide-sm">Rotate</span>
                            </ConfirmButton>
                          ) : null}
                        </div>
                      ) : canManage ? (
                        <div className="inline-form">
                          <button className="btn" onClick={() => void run(() => api.rotateInvite(teamId))} disabled={busy}>
                            Create link
                          </button>
                        </div>
                      ) : (
                        <p className="muted invite-none">No invite link</p>
                      )}
                    </div>
                    <div className="invite-row">
                      <span className="invite-label">Email</span>
                      <InviteByEmail teamId={teamId} />
                    </div>
                  </div>
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
                  <button className="btn" type="submit" disabled={busy || !handle.trim() || handle === me.user.handle}>
                    {saved === 'handle' ? 'Saved' : 'Save'}
                  </button>
                </form>
              </section>

              <section className="block">
                <h2 className="block-title">Join an org</h2>
                <JoinForm refreshMe={refreshMe} />
              </section>

              {!isOwner ? (
                <section className="block">
                  <ConfirmButton className="btn btn-danger" disabled={busy} question={`Leave ${team.name}?`} confirmLabel="Leave" danger onConfirm={leave}>
                    <Icon name="logOut" size={14} />
                    Leave org
                  </ConfirmButton>
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

const sentence = (m: string) => (m ? m.charAt(0).toUpperCase() + m.slice(1) : m)

/** Email someone a join link. Any member can; the server rate-limits and checks the cap. */
function InviteByEmail({ teamId }: { teamId: string }) {
  const [email, setEmail] = useState('')
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    const addr = email.trim()
    if (!addr || busy) return
    setBusy(true)
    setNote(null)
    try {
      await api.inviteEmail(teamId, addr)
      setEmail('')
      setNote({ ok: true, text: `Invite sent to ${addr}` })
    } catch (err) {
      setNote({ ok: false, text: sentence(err instanceof Error ? err.message : 'Could not send the invite') })
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="invite-email">
      <form className="inline-form" onSubmit={submit} noValidate>
        <input
          className="field"
          type="email"
          inputMode="email"
          autoComplete="off"
          spellCheck={false}
          placeholder="name@company.com"
          aria-label="Email to invite"
          value={email}
          maxLength={254}
          onChange={(e) => {
            setEmail(e.target.value)
            if (note) setNote(null)
          }}
        />
        <button className="btn" type="submit" disabled={busy || !email.trim()}>
          <Icon name="mail" size={14} />
          {busy ? 'Sending' : 'Invite'}
        </button>
      </form>
      <p className={`form-note ${note ? (note.ok ? 'ok' : 'bad') : ''}`} role="status" aria-live="polite">
        {note ? (
          <>
            <Icon name={note.ok ? 'check' : 'x'} size={12} />
            {note.text}
          </>
        ) : null}
      </p>
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
        className="btn btn-ghost btn-xs btn-icon"
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
