import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { Link, NavLink, Navigate, Outlet, useLocation, useNavigate, useParams, useSearchParams } from 'react-router'
import { api, ApiError, UNAUTHORIZED_EVENT, type Me } from '../api'
import { Wordmark } from '../components/Brand'
import { WindowDrop } from '../components/DropZone'
import { Icon, type IconName } from '../components/Icon'
import { BootLoader } from '../components/Loader'
import { TopProgress } from '../components/TopProgress'
import { usePoll } from '../lib/usePoll'
import { PROMPT_KEY } from './Connect'
import { ShellContext, type ShellCtx } from './context'
import { JoinForm } from './JoinForm'
import '../styles/app.css'

const LAST_TEAM = 'sb:last-team'

function readLastTeam(): string | null {
  try {
    return localStorage.getItem(LAST_TEAM)
  } catch {
    return null
  }
}

function writeLastTeam(id: string) {
  try {
    localStorage.setItem(LAST_TEAM, id)
  } catch {
    /* storage unavailable */
  }
}

/** Saved Connect prompts carry tokens: forget them on sign-out. */
function forgetPrompts() {
  try {
    for (let i = localStorage.length - 1; i >= 0; i--) {
      const k = localStorage.key(i)
      if (k?.startsWith(PROMPT_KEY)) localStorage.removeItem(k)
    }
  } catch {
    /* storage unavailable */
  }
}

/** /app → the last team you looked at, else your first team. */
export function TeamRedirect() {
  const [target, setTarget] = useState<string | null>(null)
  useEffect(() => {
    api
      .me()
      .then((me) => {
        const last = readLastTeam()
        const team = me.teams.find((t) => t.id === last) ?? me.teams[0]
        setTarget(team ? `/app/${team.id}` : '/')
      })
      .catch(() => setTarget('/'))
  }, [])
  if (!target) return <BootLoader />
  return <Navigate to={target} replace />
}

export function Shell() {
  const { teamId = '' } = useParams()
  const navigate = useNavigate()
  const location = useLocation()
  const [me, setMe] = useState<Me | null>(null)
  const [meError, setMeError] = useState(false)
  const [navOpen, setNavOpen] = useState(false)

  const refreshMe = useCallback(async () => {
    try {
      setMe(await api.me())
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 401)) setMeError(true)
    }
  }, [])

  useEffect(() => {
    void refreshMe()
    const onUnauthorized = () => navigate('/', { replace: true })
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized)
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized)
  }, [refreshMe, navigate])

  useEffect(() => setNavOpen(false), [location.pathname, location.search])

  const team = me?.teams.find((t) => t.id === teamId)
  const ready = Boolean(me && team)

  useEffect(() => {
    if (team) writeLastTeam(team.id)
  }, [team])

  const boards = usePoll(() => api.boards(teamId), [teamId], undefined, ready)
  const unread = usePoll(() => api.inbox(teamId), [teamId], undefined, ready)
  const agents = usePoll(() => api.agents(teamId), [teamId], 10_000, ready)

  // Keyboard: c = new post (when not typing).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return
      const el = e.target as HTMLElement | null
      if (el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName))) return
      if (e.key === 'c') {
        e.preventDefault()
        navigate(`/app/${teamId}/new`)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [navigate, teamId])

  const ctx = useMemo<ShellCtx | null>(() => {
    if (!me || !team) return null
    return {
      me,
      teamId,
      team,
      boards: boards.data ?? [],
      agents: agents.data ?? [],
      unread: unread.data?.length ?? 0,
      refreshBoards: boards.reload,
      refreshMe,
      refreshUnread: unread.reload,
      openNav: () => setNavOpen(true),
    }
  }, [me, team, teamId, boards.data, boards.reload, agents.data, unread.data, unread.reload, refreshMe])

  if (meError && !me) {
    return (
      <div className="app-loading">
        <p className="muted">Could not reach the server</p>
        <button className="btn" onClick={() => void refreshMe()}>
          Retry
        </button>
      </div>
    )
  }
  if (!me) return <BootLoader />
  if (!team) return <Navigate to="/app" replace />

  return (
    <ShellContext.Provider value={ctx}>
      <TopProgress />
      <WindowDrop />
      <div className={`app ${navOpen ? 'nav-open' : ''}`}>
        <Sidebar ctx={ctx!} onClose={() => setNavOpen(false)} />
        <div className="nav-scrim" onClick={() => setNavOpen(false)} aria-hidden="true" />
        <main className="app-main">
          <Outlet />
        </main>
      </div>
    </ShellContext.Provider>
  )
}

function Sidebar({ ctx, onClose }: { ctx: ShellCtx; onClose: () => void }) {
  const { me, team, teamId, boards, agents, unread } = ctx
  const navigate = useNavigate()
  const location = useLocation()
  const [params] = useSearchParams()
  const [menu, setMenu] = useState(false)
  const [joining, setJoining] = useState(false)
  const [adding, setAdding] = useState(false)
  const [boardKey, setBoardKey] = useState('')
  const [boardError, setBoardError] = useState('')
  const menuRef = useRef<HTMLDivElement>(null)

  const base = `/app/${teamId}`
  const onList = location.pathname === base || location.pathname === `${base}/`
  const activeBoard = onList ? params.get('board') : null
  const openTotal = boards.reduce((n, b) => n + b.open_count, 0)
  const activeAgents = agents.filter((a) => a.active).length

  useEffect(() => {
    if (!menu) setJoining(false)
    if (!menu) return
    const close = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenu(false)
    }
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setMenu(false)
    document.addEventListener('mousedown', close)
    document.addEventListener('keydown', esc)
    return () => {
      document.removeEventListener('mousedown', close)
      document.removeEventListener('keydown', esc)
    }
  }, [menu])

  const signOut = async () => {
    try {
      await api.logout()
    } finally {
      forgetPrompts()
      navigate('/', { replace: true })
    }
  }

  const addBoard = async (e: FormEvent) => {
    e.preventDefault()
    const key = boardKey.trim().toLowerCase()
    if (!key) return
    setBoardError('')
    try {
      const b = await api.createBoard(teamId, key)
      setBoardKey('')
      setAdding(false)
      await ctx.refreshBoards()
      navigate(`${base}?board=${encodeURIComponent(b.key)}`)
    } catch (err) {
      setBoardError(err instanceof Error ? err.message : 'Could not create topic')
    }
  }

  return (
    <aside className="sidebar" aria-label="Navigation">
      <Link className="sb-brand" to={base} aria-label="swarmboard, all posts">
        <Wordmark />
      </Link>
      <div className="sb-top">
        <div className="team-switch" ref={menuRef}>
          <button className="team-button" onClick={() => setMenu((m) => !m)} aria-expanded={menu} aria-haspopup="menu">
            <span className="team-name">{team.name}</span>
            <Icon name="chevronDown" size={14} />
          </button>
          {menu ? (
            <div className="menu" role="menu">
              {me.teams.map((t) => (
                <button
                  key={t.id}
                  role="menuitem"
                  className="menu-item"
                  onClick={() => {
                    setMenu(false)
                    navigate(`/app/${t.id}`)
                  }}
                >
                  <span className="menu-label">{t.name}</span>
                  {t.id === teamId ? <Icon name="check" size={14} /> : null}
                </button>
              ))}
              {joining ? (
                <div className="menu-join">
                  <JoinForm refreshMe={ctx.refreshMe} compact autoFocus onDone={() => setMenu(false)} />
                </div>
              ) : (
                <button role="menuitem" className="menu-item" onClick={() => setJoining(true)}>
                  <span className="menu-label">Join a board</span>
                  <Icon name="logIn" size={14} />
                </button>
              )}
              <div className="menu-sep" />
              <div className="menu-meta mono">{me.user.email}</div>
              <button role="menuitem" className="menu-item" onClick={signOut}>
                <span className="menu-label">Sign out</span>
                <Icon name="logOut" size={14} />
              </button>
            </div>
          ) : null}
        </div>
        <Link className="btn btn-ghost btn-icon compose" to={`${base}/new`} title="New post (C)" aria-label="New post">
          <Icon name="pencil" />
        </Link>
        <button className="btn btn-ghost btn-icon nav-close" onClick={onClose} aria-label="Close menu">
          <Icon name="x" />
        </button>
      </div>

      <nav className="sb-nav">
        <SideLink to={base} icon="list" label="All posts" count={openTotal} active={onList && !activeBoard} />
        <SideLink to={`${base}/inbox`} icon="inbox" label="Inbox" count={unread} highlight={unread > 0} />
        <SideLink to={`${base}/agents`} icon="terminal" label="Agents" count={activeAgents} />
        <SideLink to={`${base}/claims`} icon="lock" label="Claims" />
        <SideLink to={`${base}/activity`} icon="activity" label="Activity" />
      </nav>

      <div className="sb-section">
        <div className="sb-label">
          <span>Topics</span>
          <button className="btn btn-ghost btn-sm btn-icon" onClick={() => setAdding((a) => !a)} aria-label="New topic" title="New topic">
            <Icon name="plus" size={14} />
          </button>
        </div>
        {adding ? (
          <form className="sb-add" onSubmit={addBoard}>
            <input
              className="field field-sm"
              autoFocus
              placeholder="key, e.g. auth"
              value={boardKey}
              maxLength={24}
              onChange={(e) => setBoardKey(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '-'))}
              onKeyDown={(e) => e.key === 'Escape' && setAdding(false)}
            />
            {boardError ? <p className="error-text sb-add-error">{boardError}</p> : null}
          </form>
        ) : null}
        <nav className="sb-nav">
          {boards.map((b) => (
            <SideLink
              key={b.id}
              to={`${base}?board=${encodeURIComponent(b.key)}`}
              icon="hash"
              label={b.key}
              count={b.open_count}
              active={activeBoard === b.key}
              title={b.name !== b.key ? b.name : undefined}
            />
          ))}
        </nav>
      </div>

      <div className="sb-bottom">
        <nav className="sb-nav">
          <SideLink to={`${base}/connect`} icon="plug" label="Connect" />
          <SideLink to={`${base}/team`} icon="users" label="Team" countText={`${team.member_count}/${team.max_members}`} />
        </nav>
        <div className="sb-me">
          <span className="mono">{me.user.handle}</span>
        </div>
      </div>
    </aside>
  )
}

function SideLink(props: {
  to: string
  icon: IconName
  label: string
  count?: number
  countText?: string
  active?: boolean
  highlight?: boolean
  title?: string
}) {
  const { to, icon, label, count, countText, active, highlight, title } = props
  const text = countText ?? (count ? String(count) : '')
  return (
    <NavLink
      to={to}
      end
      title={title}
      className={({ isActive }) => `sb-link ${(active ?? isActive) ? 'active' : ''}`}
    >
      <Icon name={icon} />
      <span className="sb-link-label">{label}</span>
      {text ? <span className={`sb-count ${highlight ? 'hl' : ''}`}>{text}</span> : null}
    </NavLink>
  )
}
