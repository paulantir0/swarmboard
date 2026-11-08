import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { api, type CreatedToken } from '../api'
import { CopyButton, Snippet } from '../components/CopyButton'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { ago, fullDate, shortDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { ViewHeader } from './ViewHeader'

/** The last prompt you made stays on the page (per team, this browser). Cleared on revoke and sign-out. */
export const PROMPT_KEY = 'sb:prompt:'

function readPrompt(teamId: string): CreatedToken | null {
  try {
    const raw = localStorage.getItem(PROMPT_KEY + teamId)
    return raw ? (JSON.parse(raw) as CreatedToken) : null
  } catch {
    return null
  }
}

function writePrompt(teamId: string, t: CreatedToken | null) {
  try {
    if (t) localStorage.setItem(PROMPT_KEY + teamId, JSON.stringify(t))
    else localStorage.removeItem(PROMPT_KEY + teamId)
  } catch {
    /* storage unavailable: the prompt just won't survive a reload */
  }
}

function browserName(): string {
  const ua = navigator.userAgent
  if (/Edg\//.test(ua)) return 'Edge'
  if (/OPR\//.test(ua)) return 'Opera'
  if (/Firefox\//.test(ua)) return 'Firefox'
  if (/Chrome\//.test(ua)) return 'Chrome'
  if (/Safari\//.test(ua)) return 'Safari'
  return 'Browser'
}

function defaultLabel(): string {
  const date = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(new Date())
  return `${browserName()} · ${date}`
}

/** Older servers don't send agent_prompt; build a shorter one from the snippets. */
function promptOf(t: CreatedToken): string {
  if (t.setup.agent_prompt) return t.setup.agent_prompt
  return [
    'Set up the swarmboard MCP server for every session.',
    `If you are Claude Code, run:\n${t.setup.claude_code}`,
    `If you are Codex, apply:\n${t.setup.codex}`,
    `Then add this to my global agent instructions:\n${t.setup.agents_md}`,
  ].join('\n\n')
}

export function Connect() {
  const { teamId } = useShell()
  const tokens = usePoll(() => api.tokens(teamId), [teamId])
  const placeholder = useMemo(defaultLabel, [])
  const [label, setLabel] = useState('')
  const [created, setCreated] = useState<CreatedToken | null>(() => readPrompt(teamId))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  // A token we just made isn't in the list until the next fetch; don't mistake it for revoked.
  const fresh = useRef<string | null>(null)

  useEffect(() => setCreated(readPrompt(teamId)), [teamId])

  // Drop a remembered prompt whose token is gone (revoked elsewhere).
  useEffect(() => {
    if (!created || !tokens.data) return
    if (tokens.data.some((t) => t.id === created.id)) {
      if (fresh.current === created.id) fresh.current = null
      return
    }
    if (fresh.current === created.id) return
    writePrompt(teamId, null)
    setCreated(null)
  }, [tokens.data, created, teamId])

  const create = async (e?: FormEvent) => {
    e?.preventDefault()
    if (busy) return
    setBusy(true)
    setError('')
    try {
      const t = await api.createToken(teamId, label.trim() || placeholder)
      fresh.current = t.id
      writePrompt(teamId, t)
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
      if (created?.id === id) {
        writePrompt(teamId, null)
        setCreated(null)
      }
      void tokens.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not revoke')
    }
  }

  const reset = () => {
    writePrompt(teamId, null)
    setCreated(null)
  }

  const prompt = created ? promptOf(created) : ''

  return (
    <div className="view">
      <ViewHeader title="Connect" />
      <div className="page connect">
        <section className="connect-step" aria-label="Agent prompt">
          <div className="connect-head">
            <h2 className="connect-title">Paste this into Claude Code or Codex.</h2>
            {created ? <CopyButton text={prompt} label="Copy prompt" className="btn-primary btn-sm-h connect-copy" /> : null}
          </div>

          {created ? (
            <>
              <pre className="prompt-body" tabIndex={0} aria-label="Agent prompt">
                <code>{prompt}</code>
              </pre>
              <div className="prompt-foot">
                <Icon name="lock" size={12} />
                <span className="prompt-token mono">{created.prefix}…</span>
                <span className="muted">{created.label}</span>
                <span className="spacer" />
                <button className="btn btn-ghost btn-sm" onClick={reset}>
                  New token
                </button>
              </div>
            </>
          ) : (
            <form className="prompt-empty" onSubmit={create}>
              <input
                className="field prompt-label"
                placeholder={placeholder}
                aria-label="Token label"
                value={label}
                maxLength={60}
                onChange={(e) => setLabel(e.target.value)}
              />
              <button className="btn btn-primary btn-sm-h" type="submit" disabled={busy}>
                <Icon name="plug" size={14} />
                {busy ? 'Creating' : 'Create prompt'}
              </button>
            </form>
          )}
          {error ? <p className="error-text">{error}</p> : null}
        </section>

        {created ? (
          <details className="disclosure">
            <summary>
              <Icon name="chevronRight" size={14} />
              Manual setup
            </summary>
            <div className="disclosure-body">
              <Snippet title="Claude Code" text={created.setup.claude_code} />
              <Snippet title="Codex" hint="~/.codex/config.toml" text={created.setup.codex} />
              <Snippet title="Hooks" hint="~/.claude/settings.json" text={created.setup.claude_hooks} />
              <Snippet title="AGENTS.md" hint="or CLAUDE.md" text={created.setup.agents_md} />
              <div className="token-line">
                <span className="muted">Token</span>
                <code className="token-value">{created.token}</code>
                <CopyButton text={created.token} />
              </div>
              <div className="token-line">
                <span className="muted">MCP URL</span>
                <code className="token-value">{created.setup.mcp_url}</code>
                <CopyButton text={created.setup.mcp_url} />
              </div>
            </div>
          </details>
        ) : null}

        <section className="block">
          <div className="block-head">
            <h2 className="block-title">Tokens</h2>
            {tokens.data?.length ? <span className="mono muted">{tokens.data.length}</span> : null}
          </div>
          <Loadable ready={Boolean(tokens.data) || Boolean(tokens.error)} label="Loading tokens" className="loadable-inline">
            {tokens.error && !tokens.data ? (
              <p className="muted">{tokens.error.message}</p>
            ) : (tokens.data ?? []).length === 0 ? (
              <p className="muted">No tokens</p>
            ) : (
              <ul className="token-list">
                {(tokens.data ?? []).map((t) => (
                  <li key={t.id} className={`token-row ${created?.id === t.id ? 'current' : ''}`}>
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
          </Loadable>
        </section>
      </div>
    </div>
  )
}
