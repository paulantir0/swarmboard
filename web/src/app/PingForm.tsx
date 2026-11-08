import { useMemo, useRef, useState, type FormEvent } from 'react'
import { api } from '../api'
import { UploadList, useUploads } from '../components/MarkdownField'
import { useShell } from './context'

/** Send a ping: to "handle/agent", "agent", "@handle", "handle/*" or "@all". */
export function PingForm({ initialTo = '', postNumber, onDone, autoFocus }: { initialTo?: string; postNumber?: number; onDone?: () => void; autoFocus?: boolean }) {
  const { teamId, agents, me } = useShell()
  const [to, setTo] = useState(initialTo)
  const [body, setBody] = useState('')
  const [post, setPost] = useState(postNumber ? String(postNumber) : '')
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState('')
  const [error, setError] = useState('')
  const listId = `ping-targets-${teamId}`
  const msgRef = useRef<HTMLInputElement>(null)
  const [dragging, setDragging] = useState(false)
  const uploads = useUploads({ teamId, ref: msgRef, value: body, onChange: setBody, block: false })

  const targets = useMemo(() => {
    const set = new Set<string>()
    for (const a of agents) set.add(a.display)
    const handles = new Set(agents.map((a) => a.owner_handle))
    handles.add(me.user.handle)
    for (const h of handles) {
      set.add(`@${h}`)
      set.add(`${h}/*`)
    }
    set.add('@all')
    return [...set]
  }, [agents, me.user.handle])

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!to.trim() || !body.trim() || uploads.busy) return
    setBusy(true)
    setError('')
    setResult('')
    try {
      const n = post.trim() ? Number(post.trim().replace(/^#/, '')) : undefined
      const sent = await api.ping(teamId, to.trim(), body.trim(), Number.isFinite(n) ? n : undefined)
      setBody('')
      setResult(sent.recipients.length ? `Sent to ${sent.recipients.join(', ')}` : 'Sent')
      onDone?.()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not send')
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="ping-form" onSubmit={submit}>
      <div className="ping-fields">
        <input
          className="field ping-to mono"
          list={listId}
          placeholder="To"
          aria-label="To"
          value={to}
          autoFocus={autoFocus && !initialTo}
          onChange={(e) => setTo(e.target.value)}
        />
        <datalist id={listId}>
          {targets.map((t) => (
            <option key={t} value={t} />
          ))}
        </datalist>
        <input className="field ping-post mono" placeholder="#" aria-label="Post number" inputMode="numeric" value={post} onChange={(e) => setPost(e.target.value)} />
      </div>
      <div className="ping-fields">
        <input
          ref={msgRef}
          className={`field ping-msg ${dragging ? 'dragging' : ''}`}
          placeholder="Message"
          aria-label="Message"
          value={body}
          autoFocus={autoFocus && Boolean(initialTo)}
          maxLength={2000}
          onChange={(e) => setBody(e.target.value)}
          onPaste={uploads.onPaste}
          onDragOver={(e) => {
            if (!e.dataTransfer.types.includes('Files')) return
            e.preventDefault()
            e.dataTransfer.dropEffect = 'copy'
            setDragging(true)
          }}
          onDragLeave={() => setDragging(false)}
          onDrop={(e) => {
            setDragging(false)
            const files = [...e.dataTransfer.files]
            if (!files.length) return
            e.preventDefault()
            uploads.upload(files)
          }}
        />
        <button className="btn btn-primary btn-sm-h" type="submit" disabled={busy || uploads.busy || !to.trim() || !body.trim()}>
          Send
        </button>
      </div>
      <UploadList items={uploads.items} onDismiss={uploads.dismiss} />
      {result ? <p className="ping-result muted">{result}</p> : null}
      {error ? <p className="error-text">{error}</p> : null}
    </form>
  )
}

/** The address someone would use to ping this author back. */
export function replyTarget(a: { handle: string; agent_name: string | null }): string {
  return a.agent_name ? `${a.handle}/${a.agent_name}` : `@${a.handle}`
}
