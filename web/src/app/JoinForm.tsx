import { useEffect, useId, useRef, useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router'
import { api, ApiError, type InvitePreview } from '../api'
import { Icon } from '../components/Icon'

/** Invite code from a pasted link (any host, /join/<code>, trailing slash, query or hash) or a bare code. */
export function parseInvite(input: string): string | null {
  const s = input.trim()
  if (!s) return null
  const m = s.match(/\/join\/([A-Za-z0-9_-]{4,64})(?=$|[/?#\s])/)
  if (m) return m[1]
  return /^[A-Za-z0-9_-]{4,64}$/.test(s) ? s : null
}

function joinError(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.status === 404) return 'This invite link is invalid or was replaced. Ask for a new one.'
    if (err.status === 409) return 'That org is full. Ask an admin to make room.'
    if (err.status === 429) return 'Too many tries. Wait a minute, then try again.'
    return err.message ? err.message.charAt(0).toUpperCase() + err.message.slice(1) : 'Could not join. Try again.'
  }
  return 'Could not join. Check your connection and try again.'
}

/** Paste an invite link (or code), confirm the org, join, switch to it. */
export function JoinForm({
  refreshMe,
  compact,
  autoFocus,
  label,
  onDone,
}: {
  refreshMe: () => Promise<void>
  compact?: boolean
  autoFocus?: boolean
  /** A visible field label; without one the field is labelled for screen readers only. */
  label?: string
  onDone?: () => void
}) {
  const navigate = useNavigate()
  const fieldId = useId()
  const [value, setValue] = useState('')
  const [preview, setPreview] = useState<{ code: string; invite: InvitePreview } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const joinRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (preview && !preview.invite.full) joinRef.current?.focus()
  }, [preview])

  const join = async (code: string) => {
    setBusy(true)
    setError('')
    try {
      // An existing member gets their team_id back too, so this also covers "already in it".
      const res = await api.acceptInvite(code)
      await refreshMe()
      setValue('')
      setPreview(null)
      onDone?.()
      navigate(res.slug ? `/${res.slug}` : `/app/${res.team_id}`)
    } catch (err) {
      setError(joinError(err))
    } finally {
      setBusy(false)
    }
  }

  const lookup = async (e?: FormEvent) => {
    e?.preventDefault()
    if (busy) return
    const code = parseInvite(value)
    if (!code) {
      setError(value.trim() ? 'That doesn’t look like an invite link. Paste the whole link or its code.' : '')
      return
    }
    if (preview?.code === code && !preview.invite.full) return void join(code)
    setBusy(true)
    setError('')
    setPreview(null)
    try {
      setPreview({ code, invite: await api.invite(code) })
    } catch (err) {
      setError(joinError(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className={`join ${compact ? 'compact' : ''}`}>
      {label ? (
        <label className="label" htmlFor={fieldId}>
          {label}
        </label>
      ) : null}
      <form className="inline-form" onSubmit={lookup}>
        <input
          id={fieldId}
          className="field join-field"
          placeholder="Paste an invite link…"
          aria-label={label ? undefined : 'Invite link'}
          value={value}
          autoFocus={autoFocus}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => {
            setValue(e.target.value)
            setPreview(null)
            setError('')
          }}
          onKeyDown={(e) => {
            if (e.key === 'Escape' && onDone) {
              e.stopPropagation()
              onDone()
            }
          }}
        />
        {!preview ? (
          <button className={`btn ${compact ? 'btn-sm' : ''}`} type="submit" disabled={busy || !value.trim()}>
            <Icon name="logIn" size={14} />
            {busy ? 'Checking…' : 'Join'}
          </button>
        ) : null}
      </form>
      {preview ? (
        <div className="join-confirm" role="status">
          {preview.invite.full ? (
            <span className="join-q">
              <strong>{preview.invite.team_name}</strong> is full ({preview.invite.member_count} of {preview.invite.max_members} members).
            </span>
          ) : (
            <>
              <span className="join-q">
                Join <strong>{preview.invite.team_name}</strong>? <span className="muted">Invited by</span> <span className="mono">{preview.invite.inviter_handle}</span>
              </span>
              <button ref={joinRef} className="btn btn-primary btn-xs" onClick={() => void join(preview.code)} disabled={busy}>
                {busy ? 'Joining…' : 'Join'}
              </button>
            </>
          )}
          <button className="btn btn-ghost btn-xs" onClick={() => setPreview(null)}>
            Cancel
          </button>
        </div>
      ) : null}
      {error ? (
        <p className="error-text join-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  )
}
