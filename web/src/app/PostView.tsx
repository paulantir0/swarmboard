import { useState, type ReactNode } from 'react'
import { Link, useParams } from 'react-router'
import { api, ApiError, POST_KINDS, POST_STATUSES, type Post, type PostPatch, type PostStatus } from '../api'
import { Ago } from '../components/Ago'
import { ConfirmButton } from '../components/Confirm'
import { Empty } from '../components/Empty'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { Markdown } from '../components/Markdown'
import { MarkdownField } from '../components/MarkdownField'
import { PriorityIcon, StatusIcon, Who } from '../components/Marks'
import { Select } from '../components/Select'
import { Title } from '../components/Title'
import { KIND_LABEL, PRIORITY_LABEL, STATUS_LABEL } from '../lib/labels'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { Claim, KindTag } from './PostList'
import { ViewHeader } from './ViewHeader'

const MOD_KEY = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl'

export function PostView() {
  const { teamId, base, boards, me, refreshBoards } = useShell()
  const { n = '' } = useParams()
  const number = Number(n)
  const detail = usePoll(() => api.post(teamId, number), [teamId, number])
  const [editing, setEditing] = useState(false)
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [comment, setComment] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [bodyUploading, setBodyUploading] = useState(false)
  const [commentUploading, setCommentUploading] = useState(false)

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

  const claim = (force = false) =>
    act(async () => {
      try {
        return await api.claim(teamId, number, force)
      } catch (err) {
        if (err instanceof ApiError && err.status === 409) void detail.reload()
        throw err
      }
    })

  const release = (status?: PostStatus) => act(() => api.release(teamId, number, status))

  const startEdit = () => {
    if (!post) return
    setTitle(post.title)
    setBody(post.body)
    setEditing(true)
  }

  const saveEdit = async () => {
    if (!title.trim() || bodyUploading) return
    await patch({ title: title.trim(), body })
    setEditing(false)
  }

  const sendComment = async () => {
    const text = comment.trim()
    if (!text || busy || commentUploading) return
    setBusy(true)
    setError('')
    try {
      const c = await api.comment(teamId, number, text)
      setComment('')
      detail.setData((prev) => (prev ? { post: { ...prev.post, comment_count: prev.post.comment_count + 1 }, comments: [...prev.comments, c] } : prev!))
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not reply')
    } finally {
      setBusy(false)
    }
  }

  const back = (
    <Link className="btn btn-ghost btn-sm btn-icon" to={post ? `${base}?board=${encodeURIComponent(post.board_key)}` : base} aria-label="Back">
      <Icon name="arrowLeft" />
    </Link>
  )

  if (!post) {
    return (
      <div className="view">
        <ViewHeader title={<span className="mono">#{n}</span>} back={back} />
        <Loadable ready={Boolean(detail.error)} label={`Loading #${n}`}>
          {detail.error ? (
            <Empty
              title={`Could not open #${n}`}
              hint={detail.error.message}
              action={
                <Link className="btn btn-sm" to={base}>
                  All posts
                </Link>
              }
            />
          ) : (
            <div className="empty" />
          )}
        </Loadable>
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
            <Link className="crumb" to={`${base}?board=${encodeURIComponent(post.board_key)}`}>
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

      {/* Same position as the loading branch's Loadable, so its loader crossfades into the post. */}
      <Loadable ready label={`Loading #${n}`}>
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
                dropTarget={2}
                onBusyChange={setBodyUploading}
                actions={
                  <>
                    <button className="btn btn-ghost btn-sm" onClick={() => setEditing(false)}>
                      Cancel
                    </button>
                    <button className="btn btn-primary btn-sm" onClick={saveEdit} disabled={busy || bodyUploading || !title.trim()}>
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
              <div className="detail-meta">
                <KindTag kind={post.kind} />
                <Who author={post.author} me={meId} />
                <Ago ts={post.created_at} className="muted mono" />
              </div>
              {post.body.trim() ? <Markdown source={post.body} className="detail-body" /> : null}
            </>
          )}

          <section className="thread" aria-label="Replies">
            <h3 className="section-title">
              Replies <span className="mono muted">{comments.length}</span>
            </h3>
            {comments.length ? (
              <ol className="thread-list">
                {comments.map((c) => (
                  <li className="reply" key={c.id}>
                    <div className="reply-head">
                      <Who author={c.author} me={meId} />
                      <Ago ts={c.created_at} className="muted mono" />
                    </div>
                    <Markdown source={c.body} className="reply-body" />
                  </li>
                ))}
              </ol>
            ) : null}
            <div className="reply-box">
              <MarkdownField
                teamId={teamId}
                value={comment}
                onChange={setComment}
                onSubmit={sendComment}
                placeholder="Reply"
                rows={3}
                dropTarget={1}
                onBusyChange={setCommentUploading}
                actions={
                  <>
                    <span className="kbd-hint" title={`${MOD_KEY} + Enter`}>
                      <span className="kbd">{MOD_KEY}</span>
                      <span className="kbd">↵</span>
                    </span>
                    <button className="btn btn-primary btn-sm" onClick={sendComment} disabled={busy || commentUploading || !comment.trim()}>
                      Reply
                    </button>
                  </>
                }
              />
            </div>
          </section>
        </article>

        <aside className="props" aria-label="Properties">
          <div className="prop prop-claim">
            <span className="prop-label">Claim</span>
            <div className="prop-value claim-box">
              {claimer ? (
                <>
                  <Claim author={claimer} meId={meId} />
                  {post.claimed_at ? <Ago ts={post.claimed_at} prefix="since" className="claim-since muted mono" /> : null}
                  <div className="claim-actions">
                    {mine ? (
                      <>
                        <button className="btn btn-xs" disabled={busy} onClick={() => release()}>
                          Release
                        </button>
                        <button className="btn btn-xs" disabled={busy} onClick={() => release('done')}>
                          Done
                        </button>
                      </>
                    ) : (
                      <ConfirmButton
                        className="btn btn-xs"
                        disabled={busy}
                        question={`Take over from ${claimer.display}?`}
                        confirmLabel="Take over"
                        onConfirm={() => claim(true)}
                      >
                        Take over
                      </ConfirmButton>
                    )}
                  </div>
                </>
              ) : (
                <button className="btn btn-xs claim-btn" disabled={busy} onClick={() => claim(false)}>
                  <Icon name="lock" size={12} />
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
            icon={<Icon name="tag" size={14} />}
            value={post.kind}
            options={POST_KINDS.map((k) => [k, KIND_LABEL[k]])}
            onChange={(v) => patch({ kind: v as Post['kind'] })}
            disabled={busy}
          />
          <PropSelect
            label="Topic"
            icon={<Icon name="hash" size={14} />}
            value={post.board_key}
            options={boards.map((b) => [b.key, b.key])}
            onChange={(v) => patch({ board: v })}
            disabled={busy}
          />
          <div className="prop prop-updated">
            <span className="prop-label">Updated</span>
            <span className="prop-value prop-static">
              <Icon name="clock" size={14} />
              <Ago ts={post.updated_at} className="mono" />
            </span>
          </div>
          {error ? <p className="error-text prop-error">{error}</p> : null}
        </aside>
      </div>
      </Loadable>
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
      <span className="prop-value">
        <Select variant="quiet" icon={icon} aria-label={label} value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}>
          {!has ? <option value={value}>{value}</option> : null}
          {options.map(([v, l]) => (
            <option key={v} value={v}>
              {l}
            </option>
          ))}
        </Select>
      </span>
    </label>
  )
}
