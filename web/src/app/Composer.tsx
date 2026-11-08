import { useState, type FormEvent } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
import { api, POST_KINDS, type PostKind } from '../api'
import { Icon } from '../components/Icon'
import { MarkdownField } from '../components/MarkdownField'
import { KIND_LABEL, PRIORITY_LABEL } from '../lib/labels'
import { useShell } from './context'
import { ViewHeader } from './ViewHeader'

export function Composer() {
  const { teamId, boards, refreshBoards } = useShell()
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
  const [error, setError] = useState('')

  const boardKey = (board || boards[0]?.key || '').trim().toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 24)

  const submit = async (e?: FormEvent) => {
    e?.preventDefault()
    if (!title.trim() || !boardKey || busy) return
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
      navigate(`/app/${teamId}/p/${post.number}`, { replace: true })
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not create post')
      setBusy(false)
    }
  }

  const back = (
    <Link className="btn btn-ghost btn-icon" to={`/app/${teamId}${initialBoard ? `?board=${encodeURIComponent(initialBoard)}` : ''}`} aria-label="Back">
      <Icon name="arrowLeft" />
    </Link>
  )

  return (
    <div className="view">
      <ViewHeader title="New post" back={back} />
      <form className="composer" onSubmit={submit}>
        <div className="composer-meta">
          <input
            className="field select-sm composer-topic"
            aria-label="Topic"
            placeholder="topic"
            list="composer-topics"
            value={board || boards[0]?.key || ''}
            onChange={(e) => setBoard(e.target.value)}
            required
          />
          <datalist id="composer-topics">
            {boards.map((b) => (
              <option key={b.id} value={b.key} />
            ))}
          </datalist>
          <select className="select select-sm" aria-label="Kind" value={kind} onChange={(e) => setKind(e.target.value as PostKind)}>
            {POST_KINDS.map((k) => (
              <option key={k} value={k}>
                {KIND_LABEL[k]}
              </option>
            ))}
          </select>
          <select className="select select-sm" aria-label="Priority" value={priority} onChange={(e) => setPriority(Number(e.target.value))}>
            {[0, 1, 2, 3, 4].map((p) => (
              <option key={p} value={p}>
                {PRIORITY_LABEL[p]}
              </option>
            ))}
          </select>
        </div>
        <input
          className="field field-title"
          placeholder="Title"
          aria-label="Title"
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
          placeholder="Details"
          rows={10}
          actions={
            <>
              {kind === 'task' ? (
                <label className="check">
                  <input type="checkbox" checked={claim} onChange={(e) => setClaim(e.target.checked)} />
                  Claim
                </label>
              ) : null}
              <Link className="btn" to={`/app/${teamId}`}>
                Cancel
              </Link>
              <button className="btn btn-primary" type="submit" disabled={busy || !title.trim() || !boardKey}>
                Create
              </button>
            </>
          }
        />
        {error ? <p className="error-text">{error}</p> : null}
      </form>
    </div>
  )
}
