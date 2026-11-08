import { useState, type ReactNode } from 'react'
import { Link, useParams } from 'react-router'
import { api, ApiError, POST_KINDS, POST_STATUSES, type Post, type PostPatch, type PostStatus } from '../api'
import { Icon } from '../components/Icon'
import { Markdown } from '../components/Markdown'
import { MarkdownField } from '../components/MarkdownField'
import { PriorityIcon, StatusIcon, Who } from '../components/Marks'
import { Title } from '../components/Title'
import { KIND_LABEL, PRIORITY_LABEL, STATUS_LABEL } from '../lib/labels'
import { ago, fullDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { Claim } from './PostList'
import { ViewHeader } from './ViewHeader'

const MOD_KEY = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl'

export function PostView() {
  const { teamId, boards, me, refreshBoards } = useShell()
  const { n = '' } = useParams()
  const number = Number(n)
  const detail = usePoll(() => api.post(teamId, number), [teamId, number])
  const [editing, setEditing] = useState(false)
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [comment, setComment] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const post = detail.data?.post
  const comments = detail.data?.comments ?? []
  const meId = me.user.id

  const applyPost = (p: Post) => detail.setData((prev) => ({ post: p, comments: prev?.comments ?? [] }))

  const act = async (fn: () => Promise<Post>) => {
    setBusy(true)
    setError('')
    try {
      applyPost(await fn())
      void refreshBoards()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Something went wrong')
    } finally {
      setBusy(false)
    }
  }

  const patch = (p: PostPatch) => act(() => api.updatePost(teamId, number, p))

  const claim = (force = false) => {
    if (force && post?.claimed_by && !window.confirm(`Take over from ${post.claimed_by.display}?`)) return
    return act(async () => {
      try {
        return await api.claim(teamId, number, force)
      } catch (err) {
        if (err instanceof ApiError && err.status === 409) void detail.reload()
        throw err
      }
    })
  }

  const release = (status?: PostStatus) => act(() => api.release(teamId, number, status))

  const startEdit = () => {
    if (!post) return
    setTitle(post.title)
    setBody(post.body)
    setEditing(true)
  }

  const saveEdit = async () => {
    if (!title.trim()) return
    await patch({ title: title.trim(), body })
    setEditing(false)
  }

  const sendComment = async () => {
    const text = comment.trim()
    if (!text || busy) return
    setBusy(true)
    setError('')
    try {
      const c = await api.comment(teamId, number, text)
      setComment('')
      detail.setData((prev) => (prev ? { post: { ...prev.post, comment_count: prev.post.comment_count + 1 }, comments: [...prev.comments, c] } : prev!))
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not comment')
    } finally {
      setBusy(false)
    }
  }

  const back = (
    <Link className="btn btn-ghost btn-icon" to={post ? `/app/${teamId}?board=${encodeURIComponent(post.board_key)}` : `/app/${teamId}`} aria-label="Back">
      <Icon name="arrowLeft" />
    </Link>
  )

  if (!post) {
    return (
      <div className="view">
        <ViewHeader title={<span className="mono">#{n}</span>} back={back} />
        <div className="empty">{detail.error ? <p className="muted">{detail.error.message}</p> : null}</div>
      </div>
    )
  }

  const claimer = post.claimed_by
  const mine = claimer && claimer.user_id === meId && !claimer.agent_id

  return (
    <div className="view">
      <ViewHeader
        back={back}
        title={
          <>
            <Link className="crumb" to={`/app/${teamId}?board=${encodeURIComponent(post.board_key)}`}>
              {post.board_key}
            </Link>
            <span className="crumb-sep">/</span>
            <span className="mono">#{post.number}</span>
          </>
        }
      >
        {!editing ? (
          <button className="btn btn-ghost btn-sm" onClick={startEdit}>
            <Icon name="pencil" size={14} />
            Edit
          </button>
        ) : null}
      </ViewHeader>

      <div className="detail">
        <article className="detail-main">
          {editing ? (
            <div className="edit-form">
              <input className="field field-title" value={title} onChange={(e) => setTitle(e.target.value)} aria-label="Title" autoFocus />
              <MarkdownField
                teamId={teamId}
                value={body}
                onChange={setBody}
                rows={12}
                onSubmit={saveEdit}
                actions={
                  <>
                    <button className="btn" onClick={() => setEditing(false)}>
                      Cancel
                    </button>
                    <button className="btn btn-primary" onClick={saveEdit} disabled={busy || !title.trim()}>
                      Save
                    </button>
                  </>
                }
              />
            </div>
          ) : (
            <>
              <h2 className="detail-title">
                <Title text={post.title} />
              </h2>
              <div className="detail-byline">
                <Who author={post.author} me={meId} />
                <span className="muted mono" title={fullDate(post.created_at)}>
                  {ago(post.created_at)}
                </span>
              </div>
              {post.body.trim() ? <Markdown source={post.body} className="detail-body" /> : null}
            </>
          )}

          <section className="comments" aria-label="Comments">
            <h3 className="section-title">
              Comments <span className="mono muted">{comments.length}</span>
            </h3>
            {comments.map((c) => (
              <div className="comment" key={c.id}>
                <div className="comment-head">
                  <Who author={c.author} me={meId} />
                  <span className="muted mono" title={fullDate(c.created_at)}>
                    {ago(c.created_at)}
                  </span>
                </div>
                <Markdown source={c.body} className="comment-body" />
              </div>
            ))}
            <div className="comment-box">
              <MarkdownField
                teamId={teamId}
                value={comment}
                onChange={setComment}
                onSubmit={sendComment}
                placeholder="Comment"
                rows={3}
                actions={
                  <>
                    <span className="kbd-hint muted" title={`${MOD_KEY} + Enter`}>
                      <span className="kbd">{MOD_KEY}</span>
                      <span className="kbd">↵</span>
                    </span>
                    <button className="btn btn-primary" onClick={sendComment} disabled={busy || !comment.trim()}>
                      Comment
                    </button>
                  </>
                }
              />
            </div>
          </section>
        </article>

        <aside className="props" aria-label="Properties">
          <div className="prop">
            <span className="prop-label">Claim</span>
            <div className="prop-value claim-box">
              {claimer ? (
                <>
                  <div className="claim-who">
                    <Claim author={claimer} meId={meId} />
                    {post.claimed_at ? <span className="muted mono">{ago(post.claimed_at)}</span> : null}
                  </div>
                  <div className="claim-actions">
                    {mine ? (
                      <>
                        <button className="btn btn-sm" disabled={busy} onClick={() => release()}>
                          Release
                        </button>
                        <button className="btn btn-sm" disabled={busy} onClick={() => release('done')}>
                          Done
                        </button>
                      </>
                    ) : (
                      <button className="btn btn-sm" disabled={busy} onClick={() => claim(true)}>
                        Take over
                      </button>
                    )}
                  </div>
                </>
              ) : (
                <button className="btn btn-sm" disabled={busy} onClick={() => claim(false)}>
                  Claim
                </button>
              )}
            </div>
          </div>

          <PropSelect
            label="Status"
            icon={<StatusIcon status={post.status} />}
            value={post.status}
            options={POST_STATUSES.map((s) => [s, STATUS_LABEL[s]])}
            onChange={(v) => patch({ status: v as PostStatus })}
            disabled={busy}
          />
          <PropSelect
            label="Priority"
            icon={<PriorityIcon priority={post.priority} />}
            value={String(post.priority)}
            options={[1, 2, 3, 4, 0].map((p) => [String(p), PRIORITY_LABEL[p]])}
            onChange={(v) => patch({ priority: Number(v) })}
            disabled={busy}
          />
          <PropSelect
            label="Kind"
            value={post.kind}
            options={POST_KINDS.map((k) => [k, KIND_LABEL[k]])}
            onChange={(v) => patch({ kind: v as Post['kind'] })}
            disabled={busy}
          />
          <PropSelect
            label="Topic"
            value={post.board_key}
            options={boards.map((b) => [b.key, b.key])}
            onChange={(v) => patch({ board: v })}
            disabled={busy}
          />
          <div className="prop">
            <span className="prop-label">Updated</span>
            <span className="prop-value mono muted" title={fullDate(post.updated_at)}>
              {ago(post.updated_at)}
            </span>
          </div>
          {error ? <p className="error-text prop-error">{error}</p> : null}
        </aside>
      </div>
    </div>
  )
}

function PropSelect(props: {
  label: string
  value: string
  options: [string, string][]
  onChange: (v: string) => void
  icon?: ReactNode
  disabled?: boolean
}) {
  const { label, value, options, onChange, icon, disabled } = props
  const has = options.some(([v]) => v === value)
  return (
    <label className="prop">
      <span className="prop-label">{label}</span>
      <span className="prop-value prop-select">
        {icon ? <span className="prop-icon">{icon}</span> : null}
        <select className="select select-sm select-quiet" aria-label={label} value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}>
          {!has ? <option value={value}>{value}</option> : null}
          {options.map(([v, l]) => (
            <option key={v} value={v}>
              {l}
            </option>
          ))}
        </select>
      </span>
    </label>
  )
}
