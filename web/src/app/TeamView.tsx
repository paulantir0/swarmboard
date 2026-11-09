import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react'
import { useNavigate } from 'react-router'
import { api, type Member } from '../api'
import { Confirm, ConfirmButton } from '../components/Confirm'
import { CopyButton } from '../components/CopyButton'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { Who } from '../components/Marks'
import { Empty } from '../components/Empty'
import { fullDate, shortDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { localProblem, slugify } from './NewOrg'
import { HOST } from '../lib/org'
import { ViewHeader } from './ViewHeader'

const ROLE_LABEL = { owner: 'Owner', admin: 'Admin', member: '' } as const

type Section = 'org' | 'members' | 'invite' | 'handle' | 'leave'

const sentence = (m: string) => (m ? m.charAt(0).toUpperCase() + m.slice(1) : m)
const message = (err: unknown, fallback: string) => sentence(err instanceof Error && err.message ? err.message : fallback)

/** Keep a field in step with the server value unless the user has edited it. */
function useSynced(server: string | undefined) {
  const [value, setValue] = useState(server ?? '')
  const seen = useRef(server ?? '')
  useEffect(() => {
    if (server === undefined) return
    const prev = seen.current
    seen.current = server
    setValue((v) => (v === prev ? server : v))
  }, [server])
  return [value, setValue] as const
}

export function TeamView() {
  const { teamId, me, team: summary, refreshMe } = useShell()
  const navigate = useNavigate()
  const detail = usePoll(() => api.team(teamId), [teamId])
  const team = detail.data?.team
  const [name, setName] = useSynced(team?.name)
  const [slug, setSlug] = useSynced(team?.slug)
  const [handle, setHandle] = useSynced(me.user.handle)
  const [busy, setBusy] = useState<Section | null>(null)
  const [errors, setErrors] = useState<Partial<Record<Section, string>>>({})
  const [saved, setSaved] = useState<Section | null>(null)
  const [pending, setPending] = useState<{ uid: string; kind: 'remove' | 'transfer' } | null>(null)

  const members = detail.data?.members ?? []
  const max = detail.data?.max_members ?? summary.max_members
  const myRole = members.find((m) => m.user_id === me.user.id)?.role ?? summary.role
  const isOwner = myRole === 'owner'
  const canManage = myRole === 'owner' || myRole === 'admin'
  const full = members.length >= max

  useEffect(() => {
    if (!saved) return
    const id = window.setTimeout(() => setSaved(null), 1600)
    return () => window.clearTimeout(id)
  }, [saved])

  const run = async (section: Section, fn: () => Promise<unknown>, fallback: string, ok = false) => {
    setBusy(section)
    setErrors((e) => ({ ...e, [section]: undefined }))
    try {
      await fn()
      if (ok) setSaved(section)
      await detail.reload()
    } catch (err) {
      setErrors((e) => ({ ...e, [section]: message(err, fallback) }))
    } finally {
      setBusy(null)
    }
  }

  // Org: one Save for name and URL, shown only while either differs from the saved value.
  const nextName = name.trim()
  const nextSlug = slugify(slug)
  const nameDirty = canManage && Boolean(team) && nextName !== team?.name
  const slugDirty = isOwner && Boolean(team) && nextSlug !== team?.slug
  const orgDirty = nameDirty || slugDirty
  const orgProblem = !nextName ? 'Enter a name for the org' : slugDirty ? (!nextSlug ? 'Enter a URL for the org' : localProblem(nextSlug)) : null

  const revertOrg = () => {
    if (!team) return
    setName(team.name)
    setSlug(team.slug)
    setErrors((e) => ({ ...e, org: undefined }))
  }

  const saveOrg = (e?: FormEvent) => {
    e?.preventDefault()
    if (!team || !orgDirty || orgProblem || busy) return
    void run(
      'org',
      async () => {
        if (nameDirty) await api.renameTeam(teamId, nextName)
        if (slugDirty) {
          const res = await api.setSlug(teamId, nextSlug)
          setSlug(res.team.slug)
          await refreshMe()
          navigate(`/${res.team.slug}/team`, { replace: true })
        } else {
          await refreshMe()
        }
      },
      'Could not save the org. Try again.',
      true,
    )
  }

  const nextHandle = handle.trim().toLowerCase()
  const handleDirty = nextHandle !== me.user.handle

  const saveHandle = (e?: FormEvent) => {
    e?.preventDefault()
    if (!nextHandle || !handleDirty || busy) return
    void run(
      'handle',
      async () => {
        const res = await api.updateMe(nextHandle)
        setHandle(res.user.handle)
        await refreshMe()
      },
      'Could not save your handle. Try again.',
      true,
    )
  }

  const escapeReverts = (dirty: boolean, revert: () => void) => (e: KeyboardEvent) => {
    if (e.key === 'Escape' && dirty) {
      e.preventDefault()
      revert()
    }
  }

  const leave = () =>
    run(
      'leave',
      async () => {
        await api.removeMember(teamId, me.user.id)
        await refreshMe()
        navigate('/app', { replace: true })
      },
      'Could not leave the org. Try again.',
    )

  const setRole = (m: Member, role: 'admin' | 'member') =>
    void run('members', () => api.setRole(teamId, m.user_id, role), `Could not change ${m.handle}'s role. Try again.`)

  const confirmPending = async (m: Member) => {
    const kind = pending?.kind
    setPending(null)
    if (kind === 'remove') await run('members', () => api.removeMember(teamId, m.user_id), `Could not remove ${m.handle}. Try again.`)
    else if (kind === 'transfer')
      await run(
        'members',
        async () => {
          await api.transferOwnership(teamId, m.user_id)
          await refreshMe()
        },
        'Could not transfer ownership. Try again.',
      )
  }

  const rotate = () => run('invite', () => api.rotateInvite(teamId), 'Could not make a new link. Try again.')

  const errorFor = (section: Section) =>
    errors[section] ? (
      <p className="error-text" role="alert">
        {errors[section]}
      </p>
    ) : null

  const invite = detail.data?.invite

  return (
    <div className="view">
      <ViewHeader title="Team" />
      <Loadable ready={Boolean(team) || Boolean(detail.error)} label="Loading team…">
        <div className="page">
          {!team ? (
            <Empty
              title="Could not load the team"
              hint={detail.error?.message}
              action={
                <button className="btn" onClick={() => void detail.reload()}>
                  Retry
                </button>
              }
            />
          ) : (
            <>
              <section aria-labelledby="org-title">
                <h2 id="org-title" className="section-title">
                  Org
                </h2>
                <form className="field-stack" onSubmit={saveOrg} onKeyDown={escapeReverts(orgDirty, revertOrg)}>
                  <div className="field-group">
                    <label className="label" htmlFor={canManage ? 'org-name' : undefined}>
                      Name
                    </label>
                    {canManage ? (
                      <input
                        id="org-name"
                        name="org-name"
                        className="field settings-field"
                        value={name}
                        maxLength={60}
                        autoComplete="off"
                        onChange={(e) => setName(e.target.value)}
                      />
                    ) : (
                      <p id="org-name" className="field-static">
                        {team.name}
                      </p>
                    )}
                  </div>
                  <div className="field-group">
                    <label className="label" htmlFor={isOwner ? 'org-slug' : undefined}>
                      URL
                    </label>
                    {isOwner ? (
                      <>
                        <span className="slug-input settings-field">
                          <span className="slug-host mono">{HOST}/</span>
                          <input
                            id="org-slug"
                            name="org-slug"
                            className="field mono"
                            value={slug}
                            maxLength={40}
                            spellCheck={false}
                            autoComplete="off"
                            aria-describedby="org-slug-help"
                            onChange={(e) => setSlug(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '-'))}
                            onBlur={() => setSlug((s) => slugify(s) || s)}
                          />
                        </span>
                        <p id="org-slug-help" className="help-text">
                          Lowercase letters, numbers and dashes. Links with the old URL stop working.
                        </p>
                      </>
                    ) : (
                      <div className="inline-form settings-field">
                        <code id="org-slug" className="token-value mono">
                          {HOST}/{team.slug}
                        </code>
                        <CopyButton text={`${window.location.origin}/${team.slug}`} size="md" name="Copy org URL" />
                      </div>
                    )}
                  </div>
                  {orgDirty ? (
                    <div className="form-actions">
                      <button className="btn btn-primary" type="submit" disabled={busy === 'org' || Boolean(orgProblem)}>
                        {busy === 'org' ? 'Saving…' : 'Save'}
                      </button>
                      <button className="btn btn-ghost" type="button" onClick={revertOrg}>
                        Cancel
                      </button>
                      {orgProblem ? <span className="form-hint">{orgProblem}</span> : null}
                    </div>
                  ) : saved === 'org' ? (
                    <p className="form-note ok" role="status">
                      <Icon name="check" size={12} />
                      Saved
                    </p>
                  ) : null}
                  {errorFor('org')}
                </form>
              </section>

              <section aria-labelledby="members-title">
                <h2 id="members-title" className="section-title">
                  Members
                  <span className="section-count">
                    {members.length} of {max}
                  </span>
                </h2>
                <table className="settings-table member-table">
                  <thead className="visually-hidden">
                    <tr>
                      <th scope="col">Handle</th>
                      <th scope="col">Email</th>
                      <th scope="col">Role</th>
                      <th scope="col">Joined</th>
                      <th scope="col">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    {members.map((m) => {
                      const self = m.user_id === me.user.id
                      const ownerMenu = isOwner && !self
                      const adminRemove = myRole === 'admin' && !self && m.role === 'member'
                      const asking = pending?.uid === m.user_id ? pending.kind : null
                      return (
                        <tr key={m.user_id}>
                          <td className="member-handle">
                            <Who author={{ user_id: m.user_id, handle: m.handle, agent_id: null, agent_name: null, display: m.handle }} />
                            {self ? <span className="member-you">you</span> : null}
                          </td>
                          {asking ? (
                            <td className="cell-act cell-confirm" colSpan={4}>
                              <Confirm
                                question={asking === 'remove' ? `Remove ${m.handle} from ${team.name}?` : `Make ${m.handle} the owner? You become an admin.`}
                                confirmLabel={asking === 'remove' ? 'Remove' : 'Transfer'}
                                danger
                                busy={busy === 'members'}
                                onConfirm={() => void confirmPending(m)}
                                onCancel={() => setPending(null)}
                              />
                            </td>
                          ) : (
                            <>
                              <td className="member-email" title={m.email}>
                                {m.email}
                              </td>
                              <td className="member-role">{ROLE_LABEL[m.role] ? <span className={`role-badge ${m.role}`}>{ROLE_LABEL[m.role]}</span> : null}</td>
                              <td className="member-joined" title={`Joined ${fullDate(m.joined_at)}`}>
                                {shortDate(m.joined_at)}
                              </td>
                              <td className="cell-act">
                                {ownerMenu ? (
                                  <RowMenu
                                    label={`Manage ${m.handle}`}
                                    disabled={busy === 'members'}
                                    items={[
                                      m.role === 'admin'
                                        ? { label: 'Remove admin', onSelect: () => setRole(m, 'member') }
                                        : { label: 'Make admin', onSelect: () => setRole(m, 'admin') },
                                      { label: 'Transfer ownership', onSelect: () => setPending({ uid: m.user_id, kind: 'transfer' }) },
                                      { label: 'Remove from org', danger: true, onSelect: () => setPending({ uid: m.user_id, kind: 'remove' }) },
                                    ]}
                                  />
                                ) : adminRemove ? (
                                  <button
                                    className="btn btn-sm btn-quiet-danger"
                                    disabled={busy === 'members'}
                                    onClick={() => setPending({ uid: m.user_id, kind: 'remove' })}
                                  >
                                    Remove
                                  </button>
                                ) : null}
                              </td>
                            </>
                          )}
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
                {errorFor('members')}
              </section>

              <section aria-labelledby="invite-title">
                <h2 id="invite-title" className="section-title">
                  Invite
                  {!full ? <span className="section-count">{max - members.length === 1 ? '1 spot left' : `${max - members.length} spots left`}</span> : null}
                </h2>
                {full ? (
                  <p className="section-lede">
                    {team.name} is full ({max} of {max} members). {canManage ? 'Remove someone to invite more.' : 'Ask an admin to make room.'}
                  </p>
                ) : (
                  <div className="field-stack">
                    <div className="field-group">
                      <label className="label" htmlFor={invite ? 'invite-link' : undefined}>
                        Invite link
                      </label>
                      {invite ? (
                        <div className="inline-form">
                          <input
                            id="invite-link"
                            className="field mono"
                            readOnly
                            value={invite.url}
                            aria-describedby="invite-link-help"
                            onFocus={(e) => e.target.select()}
                          />
                          <CopyButton text={invite.url} size="md" name="Copy invite link" />
                          {canManage ? (
                            <ConfirmButton
                              className="btn"
                              disabled={busy === 'invite'}
                              question="Make a new link? The old one stops working."
                              confirmLabel="Rotate"
                              onConfirm={rotate}
                            >
                              <Icon name="refresh" size={14} />
                              Rotate
                            </ConfirmButton>
                          ) : null}
                        </div>
                      ) : canManage ? (
                        <div>
                          <button className="btn" onClick={() => void rotate()} disabled={busy === 'invite'}>
                            Create link
                          </button>
                        </div>
                      ) : (
                        <p id="invite-link" className="field-static muted">
                          No invite link yet. Ask an admin to create one.
                        </p>
                      )}
                      {invite ? (
                        <p id="invite-link-help" className="help-text">
                          Anyone with the link can join while there is room.
                        </p>
                      ) : null}
                      {errorFor('invite')}
                    </div>
                    <InviteByEmail teamId={teamId} />
                  </div>
                )}
              </section>

              <section aria-labelledby="handle-title">
                <h2 id="handle-title" className="section-title">
                  Your handle
                </h2>
                <form className="field-group" onSubmit={saveHandle} onKeyDown={escapeReverts(handleDirty, () => setHandle(me.user.handle))}>
                  <label className="visually-hidden" htmlFor="handle">
                    Handle
                  </label>
                  <div className="inline-form settings-field-row">
                    <input
                      id="handle"
                      name="handle"
                      className="field mono settings-field"
                      value={handle}
                      maxLength={20}
                      spellCheck={false}
                      autoComplete="off"
                      aria-describedby="handle-help"
                      onChange={(e) => setHandle(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ''))}
                    />
                    {handleDirty ? (
                      <>
                        <button className="btn btn-primary" type="submit" disabled={busy === 'handle' || !nextHandle}>
                          {busy === 'handle' ? 'Saving…' : 'Save'}
                        </button>
                        <button className="btn btn-ghost" type="button" onClick={() => setHandle(me.user.handle)}>
                          Cancel
                        </button>
                      </>
                    ) : saved === 'handle' ? (
                      <span className="form-note ok" role="status">
                        <Icon name="check" size={12} />
                        Saved
                      </span>
                    ) : null}
                  </div>
                  <p id="handle-help" className="help-text">
                    Your name on posts and in front of your agents, in every org. Lowercase letters, numbers and dashes.
                  </p>
                  {errorFor('handle')}
                </form>
              </section>

              {!isOwner ? (
                <section aria-labelledby="leave-title">
                  <h2 id="leave-title" className="section-title">
                    Leave org
                  </h2>
                  <p className="section-lede">You lose access to {team.name}. Someone with an invite link can add you back.</p>
                  <div className="form-actions">
                    <ConfirmButton
                      className="btn btn-danger"
                      disabled={busy === 'leave'}
                      question={`Leave ${team.name}?`}
                      confirmLabel="Leave"
                      danger
                      onConfirm={leave}
                    >
                      <Icon name="logOut" size={14} />
                      Leave {team.name}
                    </ConfirmButton>
                  </div>
                  {errorFor('leave')}
                </section>
              ) : null}
            </>
          )}
        </div>
      </Loadable>
    </div>
  )
}

/** Email someone a join link. Any member can; the server rate-limits and checks the cap. */
function InviteByEmail({ teamId }: { teamId: string }) {
  const [email, setEmail] = useState('')
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    const addr = email.trim()
    if (!addr || busy) return
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(addr)) {
      setNote({ ok: false, text: 'Enter an email address like name@company.com' })
      return
    }
    setBusy(true)
    setNote(null)
    try {
      await api.inviteEmail(teamId, addr)
      setEmail('')
      setNote({ ok: true, text: `Invite sent to ${addr}` })
    } catch (err) {
      setNote({ ok: false, text: message(err, 'Could not send the invite. Try again.') })
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="field-group" onSubmit={submit} noValidate>
      <label className="label" htmlFor="invite-email">
        Invite by email
      </label>
      <div className="inline-form">
        <input
          id="invite-email"
          name="invite-email"
          className="field"
          type="email"
          inputMode="email"
          autoComplete="off"
          spellCheck={false}
          placeholder="name@company.com…"
          value={email}
          maxLength={254}
          onChange={(e) => {
            setEmail(e.target.value)
            if (note) setNote(null)
          }}
        />
        <button className="btn" type="submit" disabled={busy || !email.trim()}>
          <Icon name="mail" size={14} />
          {busy ? 'Sending…' : 'Send invite'}
        </button>
      </div>
      <p className={`form-note ${note ? (note.ok ? 'ok' : 'bad') : ''}`} role="status" aria-live="polite">
        {note ? (
          <>
            <Icon name={note.ok ? 'check' : 'x'} size={12} />
            {note.text}
          </>
        ) : null}
      </p>
    </form>
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
    const esc = (e: globalThis.KeyboardEvent) => e.key === 'Escape' && setOpen(false)
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
