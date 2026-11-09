import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { api, type CreatedToken } from '../api'
import { Ago } from '../components/Ago'
import { Confirm } from '../components/Confirm'
import { CopyButton, Snippet } from '../components/CopyButton'
import { Empty } from '../components/Empty'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { fullDate, shortDate } from '../lib/time'
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
  const [revokeError, setRevokeError] = useState('')
  const [revoking, setRevoking] = useState<string | null>(null)
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
      setError(err instanceof Error ? err.message : 'Could not create the token. Try again.')
    } finally {
      setBusy(false)
    }
  }

  const revoke = async (id: string) => {
    setRevoking(null)
    setRevokeError('')
    try {
      await api.revokeToken(id)
      if (created?.id === id) {
        writePrompt(teamId, null)
        setCreated(null)
      }
      void tokens.reload()
    } catch (err) {
      setRevokeError(err instanceof Error ? err.message : 'Could not revoke the token. Try again.')
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
      <div className="page">
        <section className="connect-step" aria-labelledby="connect-title">
          <div className="section-head">
            <h2 id="connect-title" className="section-title">
              Connect an agent
            </h2>
            <p className="section-lede">Creates a token and a setup prompt. Paste the prompt into Claude Code or Codex on the machine that runs the agent.</p>
          </div>

          {created ? (
            <div className="prompt">
              <div className="prompt-head">
                <span className="prompt-title">Setup prompt</span>
                <span className="prompt-token">
                  <Icon name="lock" size={12} />
                  <code className="mono">{created.prefix}…</code>
                  <span className="prompt-label">{created.label}</span>
                </span>
                <span className="spacer" />
                <button className="btn btn-ghost btn-sm" onClick={reset}>
                  New prompt
                </button>
                <CopyButton text={prompt} label="Copy prompt" size="sm" className="btn-primary" />
              </div>
              <pre className="prompt-body" tabIndex={0} aria-label="Setup prompt">
                <code>{prompt}</code>
              </pre>
            </div>
          ) : (
            <form className="field-group" onSubmit={create}>
              <label className="label" htmlFor="token-name">
                Name
              </label>
              <div className="inline-form">
                <input
                  id="token-name"
                  name="token-name"
                  className="field"
                  placeholder={placeholder}
                  value={label}
                  maxLength={60}
                  autoComplete="off"
                  onChange={(e) => setLabel(e.target.value)}
                />
                <button className="btn btn-primary" type="submit" disabled={busy}>
                  <Icon name="plug" size={14} />
                  {busy ? 'Creating…' : 'Create prompt'}
                </button>
              </div>
              <p className="help-text">Shown in the token list so you can tell agents apart. Leave it empty to use “{placeholder}”.</p>
            </form>
          )}
          {error ? (
            <p className="error-text" role="alert">
              {error}
            </p>
          ) : null}

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
                  <span className="token-line-label">Token</span>
                  <code className="token-value mono">{created.token}</code>
                  <CopyButton text={created.token} name="Copy token" />
                </div>
                <div className="token-line">
                  <span className="token-line-label">MCP URL</span>
                  <code className="token-value mono">{created.setup.mcp_url}</code>
                  <CopyButton text={created.setup.mcp_url} name="Copy MCP URL" />
                </div>
              </div>
            </details>
          ) : null}
        </section>

        <section aria-labelledby="tokens-title">
          <h2 id="tokens-title" className="section-title">
            Tokens
            {tokens.data?.length ? <span className="section-count">{tokens.data.length}</span> : null}
          </h2>
          <Loadable ready={Boolean(tokens.data) || Boolean(tokens.error)} label="Loading tokens…" className="loadable-inline">
            {tokens.error && !tokens.data ? (
              <Empty
                compact
                title="Could not load tokens"
                hint={tokens.error.message}
                action={
                  <button className="btn btn-sm" onClick={() => void tokens.reload()}>
                    Retry
                  </button>
                }
              />
            ) : (tokens.data ?? []).length === 0 ? (
              <Empty compact title="No tokens yet" hint="Each prompt you create carries its own token." />
            ) : (
              <table className="settings-table token-table">
                <thead>
                  <tr>
                    <th scope="col" className="token-prefix">
                      Token
                    </th>
                    <th scope="col">Name</th>
                    <th scope="col" className="token-meta">
                      Created
                    </th>
                    <th scope="col" className="token-meta">
                      Last used
                    </th>
                    <th scope="col" className="cell-act">
                      <span className="visually-hidden">Actions</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {(tokens.data ?? []).map((t) => (
                    <tr key={t.id} className={created?.id === t.id ? 'current' : undefined}>
                      <td className="token-prefix">
                        <code className="mono">{t.prefix}…</code>
                      </td>
                      <td className="token-label" title={t.label}>
                        {t.label}
                      </td>
                      {revoking === t.id ? (
                        <td className="cell-act cell-confirm" colSpan={3}>
                          <Confirm
                            question="Revoke? Agents using it lose access."
                            confirmLabel="Revoke"
                            danger
                            onConfirm={() => void revoke(t.id)}
                            onCancel={() => setRevoking(null)}
                          />
                        </td>
                      ) : (
                        <>
                          <td className="token-meta" title={fullDate(t.created_at)}>
                            {shortDate(t.created_at)}
                          </td>
                          <td className="token-meta muted">{t.last_used_at ? <Ago ts={t.last_used_at} /> : 'Never'}</td>
                          <td className="cell-act">
                            <button className="btn btn-sm btn-quiet-danger" onClick={() => setRevoking(t.id)}>
                              Revoke
                            </button>
                          </td>
                        </>
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Loadable>
          {revokeError ? (
            <p className="error-text" role="alert">
              {revokeError}
            </p>
          ) : null}
        </section>
      </div>
    </div>
  )
}
