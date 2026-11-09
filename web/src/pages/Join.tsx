import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { api, ApiError, type InvitePreview, type Me } from '../api'
import { Wordmark } from '../components/Brand'
import '../styles/app.css'

function Solo({ children }: { children: ReactNode }) {
  return (
    <main className="solo">
      <div className="solo-card">
        <Link to="/" className="solo-brand" aria-label="swarmboard">
          <Wordmark />
        </Link>
        {children}
      </div>
    </main>
  )
}

export function Join() {
  const { code = '' } = useParams()
  const navigate = useNavigate()
  const [invite, setInvite] = useState<InvitePreview | null>(null)
  const [loadError, setLoadError] = useState<'gone' | 'failed' | null>(null)
  // undefined while loading, null when signed out.
  const [me, setMe] = useState<Me | null | undefined>(undefined)
  const [email, setEmail] = useState('')
  const [sent, setSent] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const load = useCallback(() => {
    setLoadError(null)
    api
      .invite(code)
      .then(setInvite)
      .catch((err) => setLoadError(err instanceof ApiError && err.status === 404 ? 'gone' : 'failed'))
    api
      .me()
      .then(setMe)
      .catch(() => setMe(null))
  }, [code])

  useEffect(load, [load])

  const join = async () => {
    setBusy(true)
    setError('')
    try {
      const res = await api.acceptInvite(code)
      navigate(res.slug ? `/${res.slug}` : `/app/${res.team_id}`, { replace: true })
    } catch (err) {
      setError(
        err instanceof ApiError && err.status === 409
          ? 'This org is full. Ask whoever invited you to make room.'
          : 'Could not join. Check your connection and try again.',
      )
      setBusy(false)
    }
  }

  const sendLink = async (e: FormEvent) => {
    e.preventDefault()
    const addr = email.trim()
    if (!addr || busy) return
    setBusy(true)
    setError('')
    try {
      await api.requestLink(addr, code)
      setSent(addr)
    } catch (err) {
      setError(err instanceof ApiError && err.status === 429 ? 'Too many tries. Wait a few minutes, then try again.' : 'Could not send the link. Try again.')
    } finally {
      setBusy(false)
    }
  }

  if (loadError === 'gone') {
    return (
      <Solo>
        <h1 className="solo-title">This invite link is no longer valid</h1>
        <p className="solo-body">It was replaced or the org was deleted. Ask whoever sent it for a new link.</p>
        <div className="solo-actions">
          <Link className="btn" to="/">
            Go to swarmboard
          </Link>
        </div>
      </Solo>
    )
  }

  if (loadError === 'failed') {
    return (
      <Solo>
        <h1 className="solo-title">Could not load this invite</h1>
        <p className="solo-body">Check your connection and try again.</p>
        <div className="solo-actions">
          <button className="btn btn-primary" onClick={load}>
            Retry
          </button>
        </div>
      </Solo>
    )
  }

  if (!invite || me === undefined) {
    return (
      <Solo>
        <p className="solo-body" role="status">
          Loading invite…
        </p>
      </Solo>
    )
  }

  const mine = me?.teams.find((t) => (invite.team_slug ? t.slug === invite.team_slug : t.name === invite.team_name))
  const meta = (
    <p className="solo-meta">
      Invited by <span className="mono">{invite.inviter_handle}</span> · {invite.member_count} of {invite.max_members} members
    </p>
  )

  if (mine) {
    return (
      <Solo>
        <h1 className="solo-title">{invite.team_name}</h1>
        {meta}
        <p className="solo-body">You’re already a member.</p>
        <div className="solo-actions">
          <Link className="btn btn-primary" to={`/${mine.slug}`}>
            Open {mine.name}
          </Link>
        </div>
      </Solo>
    )
  }

  return (
    <Solo>
      <h1 className="solo-title">{invite.full ? invite.team_name : `Join ${invite.team_name}`}</h1>
      {meta}
      {invite.full ? (
        <p className="solo-body">This org is full. Ask {invite.inviter_handle} to make room, then open the link again.</p>
      ) : me ? (
        <div className="solo-actions">
          <button className="btn btn-primary" onClick={join} disabled={busy}>
            {busy ? 'Joining…' : `Join ${invite.team_name}`}
          </button>
        </div>
      ) : sent ? (
        <p className="solo-body" role="status">
          Check your email. We sent a sign-in link to <strong>{sent}</strong>; opening it joins you to {invite.team_name}.
        </p>
      ) : (
        <form className="solo-form" onSubmit={sendLink}>
          <div className="field-group">
            <label htmlFor="email" className="label">
              Email
            </label>
            <input
              id="email"
              name="email"
              className="field"
              type="email"
              inputMode="email"
              autoComplete="email"
              spellCheck={false}
              placeholder="you@company.com…"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
            <p className="help-text">We’ll email you a sign-in link that joins you to {invite.team_name}.</p>
          </div>
          <div className="solo-actions">
            <button className="btn btn-primary" type="submit" disabled={busy || !email.trim()}>
              {busy ? 'Sending…' : 'Send sign-in link'}
            </button>
          </div>
        </form>
      )}
      {error ? (
        <p className="error-text" role="alert">
          {error}
        </p>
      ) : null}
      {me && !invite.full ? (
        <p className="solo-foot">
          Signed in as <span className="solo-email">{me.user.email}</span>
        </p>
      ) : null}
    </Solo>
  )
}
