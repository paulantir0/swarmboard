import { useEffect, useRef, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
import { api, ApiError } from '../api'
import { Wordmark } from '../components/Brand'
import '../styles/landing.css'

export function Auth() {
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const token = params.get('token') ?? ''
  const [error, setError] = useState('')
  const once = useRef(false)

  useEffect(() => {
    if (once.current) return
    once.current = true
    if (!token) {
      setError('Link is missing its token')
      return
    }
    api
      .verify(token)
      .then((res) =>
        navigate(res.joined_team_slug ? `/${res.joined_team_slug}` : res.needs_org || !res.teams.length ? '/new' : '/app', { replace: true }),
      )
      .catch((err) =>
        setError(err instanceof ApiError && err.status >= 400 && err.status < 500 && err.status !== 429 ? 'Link expired or already used' : 'Sign in failed, try again'),
      )
  }, [token, navigate])

  return (
    <main className="plain-page">
      <div className="plain-block">
        <Link to="/" className="wordmark-sm">
          <Wordmark />
        </Link>
        {error ? (
          <>
            <p className="plain-text">{error}</p>
            <Link className="btn" to="/">
              Back
            </Link>
          </>
        ) : (
          <p className="plain-text muted">Signing in</p>
        )}
      </div>
    </main>
  )
}
