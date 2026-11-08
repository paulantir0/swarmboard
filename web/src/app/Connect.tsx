import { useState, type FormEvent } from 'react'
import { api, type CreatedToken } from '../api'
import { CopyButton, Snippet } from '../components/CopyButton'
import { ago, fullDate, shortDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { ViewHeader } from './ViewHeader'

export function Connect() {
  const { teamId } = useShell()
  const tokens = usePoll(() => api.tokens(teamId), [teamId])
  const [label, setLabel] = useState('')
  const [created, setCreated] = useState<CreatedToken | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const create = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError('')
    try {
      const t = await api.createToken(teamId, label.trim())
      setCreated(t)
      setLabel('')
      void tokens.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not create token')
    } finally {
      setBusy(false)
    }
  }

  const revoke = async (id: string, name: string) => {
    if (!window.confirm(`Revoke ${name}? Agents using it will be disconnected.`)) return
    try {
      await api.revokeToken(id)
      if (created?.id === id) setCreated(null)
      void tokens.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not revoke')
    }
  }

  return (
    <div className="view">
      <ViewHeader title="Connect" />
      <div className="page">
        <section className="block">
          <h2 className="block-title">New token</h2>
          <form className="inline-form" onSubmit={create}>
            <input
              className="field"
              placeholder="Label, e.g. laptop"
              aria-label="Token label"
              value={label}
              maxLength={60}
              onChange={(e) => setLabel(e.target.value)}
            />
            <button className="btn btn-primary btn-sm-h" type="submit" disabled={busy}>
              Create token
            </button>
          </form>
          {error ? <p className="error-text">{error}</p> : null}
        </section>

        {created ? (
          <section className="block created" aria-live="polite">
            <div className="block-head">
              <h2 className="block-title">{created.label || 'Token'}</h2>
              <span className="muted block-note">Shown once</span>
              <span className="spacer" />
              <button className="btn btn-sm" onClick={() => setCreated(null)}>
                Done
              </button>
            </div>
            <div className="token-line">
              <code className="token-value">{created.token}</code>
              <CopyButton text={created.token} />
            </div>
            <Snippet title="Claude Code" text={created.setup.claude_code} />
            <Snippet title="Codex" text={created.setup.codex} />
            <Snippet title="Hooks" hint="~/.claude/settings.json" text={created.setup.claude_hooks} />
            <Snippet title="AGENTS.md" hint="or CLAUDE.md" text={created.setup.agents_md} />
            <div className="token-line">
              <span className="muted">MCP URL</span>
              <code className="token-value">{created.setup.mcp_url}</code>
              <CopyButton text={created.setup.mcp_url} />
            </div>
          </section>
        ) : null}

        <section className="block">
          <h2 className="block-title">Your tokens</h2>
          {tokens.loading && !tokens.data ? null : (tokens.data ?? []).length === 0 ? (
            <p className="muted">No tokens</p>
          ) : (
            <ul className="token-list">
              {(tokens.data ?? []).map((t) => (
                <li key={t.id} className="token-row">
                  <code className="mono token-prefix">{t.prefix}…</code>
                  <span className="token-label">{t.label}</span>
                  <span className="muted token-meta" title={fullDate(t.created_at)}>
                    {shortDate(t.created_at)}
                  </span>
                  <span className="muted token-meta mono" title={t.last_used_at ? fullDate(t.last_used_at) : undefined}>
                    {t.last_used_at ? `used ${ago(t.last_used_at)}` : 'unused'}
                  </span>
                  <button className="btn btn-sm btn-danger" onClick={() => revoke(t.id, t.label || t.prefix)}>
                    Revoke
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  )
}
