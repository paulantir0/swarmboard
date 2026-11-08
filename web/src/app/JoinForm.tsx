import { useEffect, useRef, useState, type FormEvent } from 'react'
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
    if (err.status === 404) return 'Invite link is invalid or was revoked'
    if (err.status === 409) return 'That board is full'
    if (err.status === 429) return 'Too many tries, wait a minute'
    return err.message
  }
  return 'Could not join'
}

/** Paste an invite link (or code), confirm the board, join, switch to it. */
export function JoinForm({ refreshMe, compact, autoFocus, onDone }: { refreshMe: () => Promise<void>; compact?: boolean; autoFocus?: boolean; onDone?: () => void }) {
  const navigate = useNavigate()
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
      navigate(`/app/${res.team_id}`)
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
      setError(value.trim() ? 'Not an invite link' : '')
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
      <form className="inline-form" onSubmit={lookup}>
        <input
          className="field mono join-field"
          placeholder="Invite link or code"
          aria-label="Invite link"
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
          <button className="btn btn-sm-h" type="submit" disabled={busy || !value.trim()}>
            <Icon name="logIn" size={14} />
            Join
          </button>
        ) : null}
      </form>
      {preview ? (
        <div className="join-confirm" role="status">
          {preview.invite.full ? (
            <span>
              <strong>{preview.invite.team_name}</strong> is full{' '}
              <span className="mono muted">
                {preview.invite.member_count}/{preview.invite.max_members}
              </span>
            </span>
          ) : (
            <>
              <span className="join-q">
                Join <strong>{preview.invite.team_name}</strong> <span className="muted">(invited by </span>
                <span className="mono">{preview.invite.inviter_handle}</span>
                <span className="muted">)?</span>
              </span>
              <button ref={joinRef} className="btn btn-primary btn-sm" onClick={() => void join(preview.code)} disabled={busy}>
                Join
              </button>
            </>
          )}
          <button className="btn btn-ghost btn-sm" onClick={() => setPreview(null)}>
            Cancel
          </button>
        </div>
      ) : null}
      {error ? <p className="error-text join-error">{error}</p> : null}
    </div>
  )
}
