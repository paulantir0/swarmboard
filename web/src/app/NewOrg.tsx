import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router'
import { api, ApiError, type Me, type SlugCheck } from '../api'
import { Wordmark } from '../components/Brand'
import { Icon } from '../components/Icon'
import { BootLoader } from '../components/Loader'
import { JoinForm } from './JoinForm'
import { HOST, RESERVED_SLUGS } from './Shell'
import '../styles/app.css'

const SLUG_MAX = 32

/** Mirrors the server: lowercase, runs of anything else become one '-', trimmed, cut on a clean boundary. */
export function slugify(raw: string): string {
  const s = raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return s.slice(0, SLUG_MAX).replace(/-+$/, '')
}

function localProblem(slug: string): string | null {
  if (!slug) return null
  if (slug.length < 2) return 'At least 2 characters'
  if (RESERVED_SLUGS.has(slug)) return `${HOST}/${slug} is reserved`
  return null
}

/** /new: name your org (and its URL). */
export function NewOrg() {
  const navigate = useNavigate()
  const [me, setMe] = useState<Me | null>(null)
  const [name, setName] = useState('')
  const [slugInput, setSlugInput] = useState('')
  const [touched, setTouched] = useState(false)
  const [check, setCheck] = useState<SlugCheck | null>(null)
  const [checking, setChecking] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const nameRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    api
      .me()
      .then(setMe)
      .catch(() => navigate('/', { replace: true }))
  }, [navigate])

  const slug = touched ? slugify(slugInput) : slugify(name)
  const problem = localProblem(slug)

  // Live availability, debounced; stale answers are dropped.
  useEffect(() => {
    setCheck(null)
    if (!slug || problem) {
      setChecking(false)
      return
    }
    setChecking(true)
    const ctrl = new AbortController()
    const id = window.setTimeout(() => {
      api
        .checkSlug(slug, { signal: ctrl.signal })
        .then((c) => setCheck(c))
        .catch(() => setCheck(null))
        .finally(() => setChecking(false))
    }, 250)
    return () => {
      window.clearTimeout(id)
      ctrl.abort()
    }
  }, [slug, problem])

  const status = !slug ? null : problem ? { ok: false, text: problem } : checking || !check ? null : check.available ? { ok: true, text: 'Available' } : { ok: false, text: check.reason || 'Taken' }

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!name.trim() || !slug || problem || busy) return
    setBusy(true)
    setError('')
    try {
      const org = await api.createOrg(name.trim(), slug)
      navigate(`/${org.slug}`, { replace: true })
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'Could not create the org')
      setBusy(false)
    }
  }

  if (!me) return <BootLoader />

  return (
    <main className="solo">
      <div className="solo-card">
        <Link to="/" className="solo-brand" aria-label="swarmboard">
          <Wordmark />
        </Link>
        <h1 className="solo-title">Name your org</h1>

        <form className="solo-form" onSubmit={submit}>
        <label className="solo-field">
          <span className="label">Org name</span>
          <input
            ref={nameRef}
            className="field"
            value={name}
            maxLength={60}
            autoFocus
            placeholder="Acme"
            onChange={(e) => setName(e.target.value)}
          />
        </label>

        <label className="solo-field">
          <span className="label">URL</span>
          <span className="slug-input">
            <span className="slug-host mono">{HOST}/</span>
            <input
              className="field mono"
              value={touched ? slugInput : slug}
              maxLength={40}
              spellCheck={false}
              autoComplete="off"
              placeholder="acme"
              aria-label="URL slug"
              onChange={(e) => {
                setTouched(true)
                setSlugInput(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '-'))
              }}
              onBlur={() => touched && setSlugInput(slugify(slugInput))}
            />
          </span>
          <span className={`slug-status ${status ? (status.ok ? 'ok' : 'bad') : ''}`} aria-live="polite">
            {status ? (
              <>
                <Icon name={status.ok ? 'check' : 'x'} size={12} />
                {status.text}
              </>
            ) : checking ? (
              <span className="muted">Checking</span>
            ) : null}
          </span>
        </label>

        {error ? <p className="error-text">{error}</p> : null}

        <div className="solo-actions">
          {me.teams.length ? (
            <Link className="btn btn-ghost" to="/app">
              Cancel
            </Link>
          ) : null}
          <button className="btn btn-primary" type="submit" disabled={busy || !name.trim() || !slug || Boolean(problem) || check?.available === false}>
            {busy ? 'Creating' : 'Create org'}
          </button>
        </div>
        </form>

        <div className="solo-section solo-join">
          <span className="label">Have an invite link?</span>
          <JoinForm refreshMe={async () => setMe(await api.me())} />
        </div>
      </div>
    </main>
  )
}
