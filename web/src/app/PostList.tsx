import { useEffect, useMemo, useRef, useState } from 'react'
import { Link, useSearchParams } from 'react-router'
import { api, POST_KINDS, POST_STATUSES, type Post, type PostStatus } from '../api'
import { Ago } from '../components/Ago'
import { Empty } from '../components/Empty'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { PriorityIcon, StatusIcon, Who } from '../components/Marks'
import { Select } from '../components/Select'
import { Title } from '../components/Title'
import { KIND_LABEL, STATUS_LABEL, STATUS_ORDER } from '../lib/labels'
import { usePoll } from '../lib/usePoll'
import { useShell } from './context'
import { TLink, useArrivals } from './motion'
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
  const { teamId, base, boards, me } = useShell()
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

  const isNew = useArrivals(posts.data?.map((p) => p.id))

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
  const newHref = `${base}/new-post${board ? `?board=${encodeURIComponent(board)}` : ''}`

  const clearFilters = () => {
    setSearch('')
    setParams(
      (prev) => {
        const next = new URLSearchParams(prev)
        for (const k of ['kind', 'status', 'q']) next.delete(k)
        return next
      },
      { replace: true },
    )
  }

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
        <Link className="btn btn-sm new-post" to={newHref} title="New post (C)" aria-keyshortcuts="c">
          <Icon name="plus" size={14} />
          <span className="new-post-label">New post</span>
          <span className="kbd new-post-kbd" aria-hidden="true">
            C
          </span>
        </Link>
      </ViewHeader>

      <div className="toolbar filters">
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
        <Select aria-label="Topic" value={board} on={Boolean(board)} onChange={(e) => setParam('board', e.target.value)}>
          <option value="">All topics</option>
          {boards.map((b) => (
            <option key={b.id} value={b.key}>
              {b.key}
            </option>
          ))}
        </Select>
        <Select aria-label="Kind" value={kind} on={Boolean(kind)} onChange={(e) => setParam('kind', e.target.value)}>
          <option value="">All kinds</option>
          {POST_KINDS.map((k) => (
            <option key={k} value={k}>
              {KIND_LABEL[k]}
            </option>
          ))}
        </Select>
        <Select aria-label="Status" value={status} on={Boolean(status)} onChange={(e) => setParam('status', e.target.value)}>
          <option value="">Any status</option>
          <option value="active">Active</option>
          {POST_STATUSES.map((s) => (
            <option key={s} value={s}>
              {STATUS_LABEL[s]}
            </option>
          ))}
        </Select>
        {filtered ? (
          <button className="btn btn-ghost btn-xs filters-clear" onClick={clearFilters}>
            <Icon name="x" size={12} />
            Clear
          </button>
        ) : null}
      </div>

      <Loadable ready={Boolean(posts.data) || Boolean(posts.error)} label="Loading posts">
      <div className="list" role="list">
        {posts.error && !posts.data ? (
          <Empty
            title="Could not load posts"
            hint={posts.error.message}
            action={
              <button className="btn btn-sm" onClick={() => void posts.reload()}>
                Retry
              </button>
            }
          />
        ) : groups.length === 0 ? (
          filtered ? (
            <Empty
              title="No posts match these filters"
              hint="Try another search or topic, or clear the filters."
              action={
                <button className="btn btn-sm" onClick={clearFilters}>
                  Clear filters
                </button>
              }
            />
          ) : (
            <Empty
              title={board ? `Nothing in #${board} yet` : 'No posts yet'}
              hint="Agents post tasks, discoveries, decisions and handoffs here."
              action={
                <Link className="btn btn-sm" to={newHref}>
                  <Icon name="plus" size={14} />
                  New post
                </Link>
              }
            />
          )
        ) : (
          groups.map((g) => {
            const isCollapsed = collapsed.has(g.status)
            return (
              <section key={g.status} className="group">
                <button className="group-head" onClick={() => toggle(g.status)} aria-expanded={!isCollapsed}>
                  <Icon name={isCollapsed ? 'chevronRight' : 'chevronDown'} size={14} />
                  <StatusIcon status={g.status} />
                  <span className="group-title">{STATUS_LABEL[g.status]}</span>
                  <span className="group-count">{g.posts.length}</span>
                </button>
                {!isCollapsed
                  ? g.posts.map((p) => <PostRow key={p.id} post={p} base={base} showBoard={!board} meId={me.user.id} fresh={isNew(p.id)} />)
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

export function PostRow({ post, base, showBoard, meId, fresh }: { post: Post; base: string; showBoard: boolean; meId: string; fresh: boolean }) {
  const holder = post.claimed_by
  const heldByAuthor = holder && holder.user_id === post.author.user_id && holder.agent_id === post.author.agent_id
  return (
    <TLink className={`row ${fresh ? 'arrive' : ''}`} to={`${base}/p/${post.number}`} role="listitem">
      <span className="c-pri">{post.priority ? <PriorityIcon priority={post.priority} /> : null}</span>
      <span className="c-num mono">#{post.number}</span>
      <span className="c-status">
        <StatusIcon status={post.status} />
      </span>
      <span className="c-title">
        {holder ? <span className="claim-mark title-claim" title={`Claimed by ${holder.display}`} /> : null}
        <span className="title-text" title={post.title}>
          <Title text={post.title} />
        </span>
        {post.comment_count > 0 ? (
          <span className="comment-count" title={`${post.comment_count} ${post.comment_count === 1 ? 'reply' : 'replies'}`}>
            <Icon name="message" size={12} />
            {post.comment_count}
          </span>
        ) : null}
        {showBoard ? <span className="board-tag mono">{post.board_key}</span> : null}
      </span>
      <span className="c-meta">
        <span className="c-kind">
          <KindTag kind={post.kind} />
        </span>
        {holder ? (
          <span className="c-claim">
            <Claim author={holder} meId={meId} nested />
          </span>
        ) : null}
        {heldByAuthor ? null : (
          <span className="c-author">
            <Who author={post.author} me={meId} nested />
          </span>
        )}
        <Ago ts={post.updated_at} className="c-time" />
      </span>
    </TLink>
  )
}

/** The post's kind as a quiet tag; blockers carry the danger ink. */
export function KindTag({ kind }: { kind: Post['kind'] }) {
  return <span className={`kind-tag k-${kind}`}>{KIND_LABEL[kind] ?? kind}</span>
}

/** Who holds a claim: the accent dot (the claim mark) and their identity, as a chip. */
export function Claim({ author, meId, nested }: { author: Post['author']; meId: string; nested?: boolean }) {
  return (
    <span className="claim" title={`Claimed by ${author.display}`}>
      <span className="claim-mark" aria-hidden="true" />
      <Who author={author} me={meId} nested={nested} />
    </span>
  )
}
