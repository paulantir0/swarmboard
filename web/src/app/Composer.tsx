import { useState, type FormEvent } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
import { api, POST_KINDS, type PostKind } from '../api'
import { Icon } from '../components/Icon'
import { MarkdownField, SUBMIT_SHORTCUT, SubmitKey } from '../components/MarkdownField'
import { PriorityIcon } from '../components/Marks'
import { Select } from '../components/Select'
import { KIND_LABEL, PRIORITY_LABEL } from '../lib/labels'
import { useShell } from './context'
import { ViewHeader } from './ViewHeader'

export function Composer() {
  const { teamId, base, boards, refreshBoards } = useShell()
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const initialBoard = params.get('board') ?? ''
  const [board, setBoard] = useState(initialBoard)
  const [kind, setKind] = useState<PostKind>('task')
  const [priority, setPriority] = useState(0)
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [claim, setClaim] = useState(false)
  const [busy, setBusy] = useState(false)
  const [uploading, setUploading] = useState(false)
  const [error, setError] = useState('')

  const boardKey = (board || boards[0]?.key || '').trim().toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 24)

  const submit = async (e?: FormEvent) => {
    e?.preventDefault()
    if (!title.trim() || !boardKey || busy || uploading) return
    setBusy(true)
    setError('')
    try {
      if (!boards.some((b) => b.key === boardKey)) await api.createBoard(teamId, boardKey)
      const post = await api.createPost(teamId, { board: boardKey, title: title.trim(), body, kind, priority })
      if (claim) {
        try {
          await api.claim(teamId, post.number)
        } catch {
          /* the post exists; claim can be retried from the detail view */
        }
      }
      void refreshBoards()
      navigate(`${base}/p/${post.number}`, { replace: true })
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not create the post. Try again.')
      setBusy(false)
    }
  }

  const backHref = `${base}${initialBoard ? `?board=${encodeURIComponent(initialBoard)}` : ''}`
  const back = (
    <Link className="btn btn-ghost btn-sm btn-icon" to={backHref} aria-label="Back">
      <Icon name="arrowLeft" />
    </Link>
  )

  return (
    <div className="view">
      <ViewHeader title="New post" back={back} />
      <form className="page composer" onSubmit={submit}>
        <div className="composer-meta">
          <span className="pill-field">
            <Icon name="hash" size={14} />
            <input
              className="pill-input mono"
              aria-label="Topic"
              placeholder="Topic"
              list="composer-topics"
              autoComplete="off"
              spellCheck={false}
              value={board || boards[0]?.key || ''}
              onChange={(e) => setBoard(e.target.value)}
              size={Math.max(6, (board || boards[0]?.key || '').length + 1)}
              required
            />
          </span>
          <datalist id="composer-topics">
            {boards.map((b) => (
              <option key={b.id} value={b.key} />
            ))}
          </datalist>
          <Select aria-label="Kind" icon={<Icon name="tag" size={14} />} value={kind} onChange={(e) => setKind(e.target.value as PostKind)}>
            {POST_KINDS.map((k) => (
              <option key={k} value={k}>
                {KIND_LABEL[k]}
              </option>
            ))}
          </Select>
          <Select aria-label="Priority" icon={<PriorityIcon priority={priority} />} value={priority} onChange={(e) => setPriority(Number(e.target.value))}>
            {[0, 1, 2, 3, 4].map((p) => (
              <option key={p} value={p}>
                {PRIORITY_LABEL[p]}
              </option>
            ))}
          </Select>
        </div>
        <input
          className="field field-title"
          placeholder="Title"
          aria-label="Title"
          name="title"
          autoComplete="off"
          value={title}
          autoFocus
          maxLength={200}
          onChange={(e) => setTitle(e.target.value)}
        />
        <MarkdownField
          teamId={teamId}
          value={body}
          onChange={setBody}
          onSubmit={() => void submit()}
          label="Details"
          hideLabel
          id="post-body"
          placeholder="Details"
          rows={10}
          dropTarget={1}
          onBusyChange={setUploading}
          actions={
            <>
              {kind === 'task' ? (
                <label className="check">
                  <input type="checkbox" checked={claim} onChange={(e) => setClaim(e.target.checked)} />
                  Claim it
                </label>
              ) : null}
              <Link className="btn btn-ghost btn-sm" to={backHref}>
                Cancel
              </Link>
              <button
                className="btn btn-primary btn-sm"
                type="submit"
                disabled={busy || uploading || !title.trim() || !boardKey}
                aria-keyshortcuts={SUBMIT_SHORTCUT}
              >
                {busy ? 'Creating…' : 'Create post'}
                <SubmitKey />
              </button>
            </>
          }
        />
        {error ? (
          <p className="error-text" role="alert">
            {error}
          </p>
        ) : null}
      </form>
    </div>
  )
}
