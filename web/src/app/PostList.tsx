import { useEffect, useMemo, useRef, useState } from 'react'
import { Link, useSearchParams } from 'react-router'
import { api, POST_KINDS, POST_STATUSES, type Post, type PostStatus } from '../api'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { PriorityIcon, StatusIcon, Who } from '../components/Marks'
import { Title } from '../components/Title'
import { KIND_LABEL, STATUS_LABEL, STATUS_ORDER } from '../lib/labels'
import { ago } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { ViewHeader } from './ViewHeader'

const COLLAPSE_KEY = 'sb:collapsed-groups'

function readCollapsed(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSE_KEY)
    if (raw) return new Set(JSON.parse(raw) as string[])
  } catch {
    /* ignore */
  }
  return new Set(['done', 'closed'])
}

function writeCollapsed(s: Set<string>) {
  try {
    localStorage.setItem(COLLAPSE_KEY, JSON.stringify([...s]))
  } catch {
    /* ignore */
  }
}

export function PostList() {
  const { teamId, boards, me } = useShell()
  const [params, setParams] = useSearchParams()
  const board = params.get('board') ?? ''
  const kind = params.get('kind') ?? ''
  const status = params.get('status') ?? ''
  const q = params.get('q') ?? ''
  const [search, setSearch] = useState(q)
  const [collapsed, setCollapsed] = useState(readCollapsed)
  const searchRef = useRef<HTMLInputElement>(null)

  useEffect(() => setSearch(q), [q])

  // Debounce search into the URL.
  useEffect(() => {
    if (search === q) return
    const id = window.setTimeout(() => setParam('q', search.trim()), 250)
    return () => window.clearTimeout(id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search])

  // "/" focuses search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null
      if (el && /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) return
      if (e.key === '/') {
        e.preventDefault()
        searchRef.current?.focus()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const setParam = (key: string, value: string) => {
    setParams(
      (prev) => {
        const next = new URLSearchParams(prev)
        if (value) next.set(key, value)
        else next.delete(key)
        return next
      },
      { replace: true },
    )
  }

  const posts = usePoll(
    () => api.posts(teamId, { board: board || undefined, kind: kind || undefined, status: status || undefined, q: q || undefined, limit: 200 }),
    [teamId, board, kind, status, q],
  )

  const groups = useMemo(() => {
    const by = new Map<PostStatus, Post[]>()
    for (const p of posts.data ?? []) {
      const list = by.get(p.status) ?? []
      list.push(p)
      by.set(p.status, list)
    }
    return STATUS_ORDER.filter((s) => by.has(s)).map((s) => ({
      status: s,
      posts: by.get(s)!.sort((a, b) => b.updated_at - a.updated_at),
    }))
  }, [posts.data])

  const toggle = (s: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(s)) next.delete(s)
      else next.add(s)
      writeCollapsed(next)
      return next
    })
  }

  const boardInfo = boards.find((b) => b.key === board)
  const filtered = Boolean(kind || status || q)
  const newHref = `/app/${teamId}/new${board ? `?board=${encodeURIComponent(board)}` : ''}`

  return (
    <div className="view">
      <ViewHeader
        title={
          board ? (
            <>
              <span className="muted">#</span>
              {board}
              {boardInfo && boardInfo.name !== boardInfo.key ? <span className="view-subtitle">{boardInfo.name}</span> : null}
            </>
          ) : (
            'All posts'
          )
        }
      >
        <Link className="btn btn-primary" to={newHref}>
          <Icon name="plus" size={14} />
          <span className="hide-sm">New post</span>
        </Link>
      </ViewHeader>

      <div className="filters">
        <div className="search">
          <Icon name="search" size={14} />
          <input
            ref={searchRef}
            className="field field-sm search-field"
            type="search"
            placeholder="Search"
            aria-label="Search posts"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          {!search ? <span className="kbd search-kbd">/</span> : null}
        </div>
        <select className="select select-sm" aria-label="Board" value={board} onChange={(e) => setParam('board', e.target.value)}>
          <option value="">All topics</option>
          {boards.map((b) => (
            <option key={b.id} value={b.key}>
              {b.key}
            </option>
          ))}
        </select>
        <select className="select select-sm" aria-label="Kind" value={kind} onChange={(e) => setParam('kind', e.target.value)}>
          <option value="">All kinds</option>
          {POST_KINDS.map((k) => (
            <option key={k} value={k}>
              {KIND_LABEL[k]}
            </option>
          ))}
        </select>
        <select className="select select-sm" aria-label="Status" value={status} onChange={(e) => setParam('status', e.target.value)}>
          <option value="">Any status</option>
          <option value="active">Active</option>
          {POST_STATUSES.map((s) => (
            <option key={s} value={s}>
              {STATUS_LABEL[s]}
            </option>
          ))}
        </select>
      </div>

      <Loadable ready={Boolean(posts.data) || Boolean(posts.error)} label="Loading posts">
      <div className="list" role="list">
        {posts.error && !posts.data ? (
          <div className="empty">
            <p className="muted">{posts.error.message}</p>
          </div>
        ) : groups.length === 0 ? (
          <div className="empty">
            <p className="muted">{filtered ? 'No matching posts' : 'No posts'}</p>
            {!filtered ? (
              <Link className="btn" to={newHref}>
                New post
              </Link>
            ) : null}
          </div>
        ) : (
          groups.map((g) => {
            const isCollapsed = collapsed.has(g.status)
            return (
              <section key={g.status} className="group">
                <button className="group-head" onClick={() => toggle(g.status)} aria-expanded={!isCollapsed}>
                  <Icon name={isCollapsed ? 'chevronRight' : 'chevronDown'} size={14} />
                  <StatusIcon status={g.status} />
                  <span className="group-title">{STATUS_LABEL[g.status]}</span>
                  <span className="group-count mono">{g.posts.length}</span>
                </button>
                {!isCollapsed
                  ? g.posts.map((p) => <PostRow key={p.id} post={p} teamId={teamId} showBoard={!board} meId={me.user.id} />)
                  : null}
              </section>
            )
          })
        )}
      </div>
      </Loadable>
    </div>
  )
}

function PostRow({ post, teamId, showBoard, meId }: { post: Post; teamId: string; showBoard: boolean; meId: string }) {
  return (
    <Link className="row" to={`/app/${teamId}/p/${post.number}`} role="listitem">
      <span className="c-pri">{post.priority ? <PriorityIcon priority={post.priority} /> : null}</span>
      <span className="c-num mono">#{post.number}</span>
      <span className="c-status">
        <StatusIcon status={post.status} />
      </span>
      <span className="c-title">
        {post.claimed_by ? <span className="claim-mark title-claim" title={`Claimed by ${post.claimed_by.display}`} /> : null}
        <span className="title-text">
          <Title text={post.title} />
        </span>
        {showBoard ? <span className="board-tag mono">{post.board_key}</span> : null}
        {post.comment_count > 0 ? (
          <span className="comment-count mono" title={`${post.comment_count} comments`}>
            <Icon name="message" size={12} />
            {post.comment_count}
          </span>
        ) : null}
      </span>
      <span className="c-kind">{KIND_LABEL[post.kind] ?? post.kind}</span>
      <span className="c-claim">{post.claimed_by ? <Claim author={post.claimed_by} meId={meId} /> : null}</span>
      <span className="c-author">
        <Who author={post.author} me={meId} />
      </span>
      <span className="c-time mono" title={new Date(post.updated_at).toLocaleString()}>
        {ago(post.updated_at)}
      </span>
    </Link>
  )
}

export function Claim({ author, meId }: { author: Post['author']; meId: string }) {
  return (
    <span className="claim" title={`Claimed by ${author.display}`}>
      <span className="claim-mark" aria-hidden="true" />
      <Who author={author} me={meId} />
    </span>
  )
}
