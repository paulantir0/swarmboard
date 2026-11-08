import { useEffect, useState, type FormEvent } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { api, ApiError, type InvitePreview } from '../api'
import { Wordmark } from '../components/Brand'
import '../styles/landing.css'

export function Join() {
  const { code = '' } = useParams()
  const navigate = useNavigate()
  const [invite, setInvite] = useState<InvitePreview | null>(null)
  const [loadError, setLoadError] = useState('')
  const [signedIn, setSignedIn] = useState<boolean | null>(null)
  const [email, setEmail] = useState('')
  const [sent, setSent] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    api
      .invite(code)
      .then(setInvite)
      .catch((err) => setLoadError(err instanceof ApiError && err.status === 404 ? 'Invite link is no longer valid' : 'Could not load invite'))
    api
      .me()
      .then(() => setSignedIn(true))
      .catch(() => setSignedIn(false))
  }, [code])

  const join = async () => {
    setBusy(true)
    setError('')
    try {
      const res = await api.acceptInvite(code)
      navigate(`/app/${res.team_id}`, { replace: true })
    } catch (err) {
      setError(err instanceof ApiError && err.status === 409 ? 'Team is full' : err instanceof Error ? err.message : 'Could not join')
      setBusy(false)
    }
  }

  const sendLink = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError('')
    try {
      await api.requestLink(email.trim(), code)
      setSent(true)
    } catch (err) {
      setError(err instanceof ApiError && err.status === 429 ? 'Too many tries, wait a few minutes' : 'Could not send, try again')
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="plain-page">
      <div className="plain-block">
        <Link to="/" className="wordmark-sm">
          <Wordmark />
        </Link>
        {loadError ? (
          <p className="plain-text">{loadError}</p>
        ) : !invite || signedIn === null ? (
          <p className="plain-text muted">Loading</p>
        ) : (
          <>
            <h1 className="join-title">{invite.team_name}</h1>
            <p className="join-meta mono muted">
              {invite.inviter_handle} · {invite.member_count}/{invite.max_members}
            </p>
            {invite.full ? (
              <p className="plain-text">Team is full</p>
            ) : signedIn ? (
              <div className="landing-row">
                <button className="btn btn-primary btn-hero" onClick={join} disabled={busy}>
                  Join
                </button>
              </div>
            ) : sent ? (
              <p className="landing-sent" role="status">
                Check your email.
              </p>
            ) : (
              <form className="landing-row" onSubmit={sendLink}>
                <label htmlFor="email" className="visually-hidden">
                  Email
                </label>
                <input
                  id="email"
                  className="field field-hero"
                  type="email"
                  inputMode="email"
                  autoComplete="email"
                  placeholder="you@example.com"
                  required
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                />
                <button className="btn btn-primary btn-hero" type="submit" disabled={busy}>
                  Join
                </button>
              </form>
            )}
            <p className="landing-error error-text" role="alert">
              {error}
            </p>
          </>
        )}
      </div>
    </main>
  )
}
