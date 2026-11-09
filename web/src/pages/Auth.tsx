import { useCallback, useEffect, useRef, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
import { api, ApiError } from '../api'
import { Wordmark } from '../components/Brand'
import '../styles/app.css'

type Failure = 'missing' | 'expired' | 'failed'

const COPY: Record<Failure, { title: string; body: string }> = {
  missing: { title: 'This sign-in link is incomplete', body: 'Open the link straight from the email, or send yourself a new one.' },
  expired: { title: 'This sign-in link has expired or was already used', body: 'Each link works once. Send yourself a new one.' },
  failed: { title: 'Could not sign you in', body: 'Check your connection and try again.' },
}

export function Auth() {
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const token = params.get('token') ?? ''
  const [failure, setFailure] = useState<Failure | null>(null)
  const once = useRef(false)

  const verify = useCallback(() => {
    setFailure(null)
    api
      .verify(token)
      .then((res) =>
        navigate(res.joined_team_slug ? `/${res.joined_team_slug}` : res.needs_org || !res.teams.length ? '/new' : '/app', { replace: true }),
      )
      .catch((err) => setFailure(err instanceof ApiError && err.status >= 400 && err.status < 500 && err.status !== 429 ? 'expired' : 'failed'))
  }, [token, navigate])

  useEffect(() => {
    if (once.current) return
    once.current = true
    if (!token) setFailure('missing')
    else verify()
  }, [token, verify])

  return (
    <main className="solo">
      <div className="solo-card">
        <Link to="/" className="solo-brand" aria-label="swarmboard">
          <Wordmark />
        </Link>
        {failure ? (
          <>
            <h1 className="solo-title">{COPY[failure].title}</h1>
            <p className="solo-body">{COPY[failure].body}</p>
            <div className="solo-actions">
              {failure === 'failed' ? (
                <>
                  <button className="btn btn-primary" onClick={verify}>
                    Retry
                  </button>
                  <Link className="btn" to="/">
                    Send a new link
                  </Link>
                </>
              ) : (
                <Link className="btn btn-primary" to="/">
                  Send a new link
                </Link>
              )}
            </div>
          </>
        ) : (
          <p className="solo-body" role="status">
            Signing you in…
          </p>
        )}
      </div>
    </main>
  )
}
