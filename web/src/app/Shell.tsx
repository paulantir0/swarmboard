import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { Link, Navigate, Outlet, useLocation, useNavigate, useParams, useSearchParams } from 'react-router'
import { api, ApiError, UNAUTHORIZED_EVENT, type Me, type TeamSummary } from '../api'
import { Wordmark } from '../components/Brand'
import { WindowDrop } from '../components/DropZone'
import { Icon, type IconName } from '../components/Icon'
import { BootLoader } from '../components/Loader'
import { TopProgress } from '../components/TopProgress'
import { usePoll } from '../lib/usePoll'
import { PROMPT_KEY } from './Connect'
import { ShellContext, type ShellCtx } from './context'
import { JoinForm } from './JoinForm'
import { NotMember } from './NotMember'
import { RESERVED_SLUGS } from '../lib/org'
import { setTitle } from '../lib/title'
import { Count, RouteView, TNavLink, reducedMotion, useArrivals } from './motion'
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

/** The org you looked at last, else your first. */
export function pickOrg(teams: TeamSummary[]): TeamSummary | undefined {
  const last = readLastTeam()
  return teams.find((t) => t.id === last) ?? teams[0]
}

/** /app → your last org, else the first, else /new. */
export function TeamRedirect() {
  const [target, setTarget] = useState<string | null>(null)
  useEffect(() => {
    api
      .me()
      .then((me) => {
        const team = pickOrg(me.teams)
        setTarget(team ? `/${team.slug}` : '/new')
      })
      .catch(() => setTarget('/'))
  }, [])
  if (!target) return <BootLoader />
  return <Navigate to={target} replace />
}

/** Old links: /app/:teamId/... → /:slug/... */
export function LegacyRedirect() {
  const { teamId = '', '*': rest = '' } = useParams()
  const location = useLocation()
  const [target, setTarget] = useState<string | null>(null)
  useEffect(() => {
    api
      .me()
      .then((me) => {
        const team = me.teams.find((t) => t.id === teamId)
        if (!team) return setTarget('/app')
        const path = rest === 'new' ? 'new-post' : rest
        setTarget(`/${team.slug}${path ? `/${path}` : ''}${location.search}`)
      })
      .catch(() => setTarget('/'))
  }, [teamId, rest, location.search])
  if (!target) return <BootLoader />
  return <Navigate to={target} replace />
}

export function Shell() {
  const { slug = '' } = useParams()
  const navigate = useNavigate()
  const location = useLocation()
  const [me, setMe] = useState<Me | null>(null)
  const [meError, setMeError] = useState(false)
  const [navOpen, setNavOpen] = useState(false)
  const [missing, setMissing] = useState<string | null>(null)
  const checked = useRef(new Set<string>())

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

  // Orgs, roles and member counts change elsewhere; refresh /api/me now and then while visible.
  useEffect(() => {
    const id = window.setInterval(() => {
      if (document.visibilityState === 'visible') void refreshMe()
    }, 30_000)
    return () => window.clearInterval(id)
  }, [refreshMe])

  const reserved = RESERVED_SLUGS.has(slug)
  const team = me?.teams.find((t) => t.slug === slug)
  const teamId = team?.id ?? ''
  const ready = Boolean(me && team)

  // Not in /api/me: maybe joined or renamed elsewhere. Ask once per slug, else show "Not a member".
  useEffect(() => {
    if (!me || team || reserved) return
    if (checked.current.has(slug)) {
      setMissing(slug)
      return
    }
    checked.current.add(slug)
    let alive = true
    api
      .org(slug)
      .then(() => refreshMe())
      .catch(() => alive && setMissing(slug))
    return () => {
      alive = false
    }
  }, [me, team, slug, reserved, refreshMe])

  useEffect(() => {
    if (team) writeLastTeam(team.id)
  }, [team])

  const boards = usePoll(() => api.boards(teamId), [teamId], undefined, ready)
  const unread = usePoll(() => api.inbox(teamId), [teamId], undefined, ready)
  const agents = usePoll(() => api.agents(teamId), [teamId], 10_000, ready)

  // Tab title from the route: "#4 · Acme", "(2) Inbox · Acme", "#auth · Acme".
  const unreadCount = unread.data?.length ?? 0
  const agentList = agents.data
  useEffect(() => {
    if (!team) return
    const rest = location.pathname.slice(slug.length + 1).replace(/^\/|\/$/g, '')
    const [section = '', id] = rest.split('/')
    const board = new URLSearchParams(location.search).get('board')
    const named: Record<string, string> = {
      inbox: 'Inbox',
      claims: 'Claims',
      activity: 'Activity',
      connect: 'Connect',
      team: 'Team',
      'new-post': 'New post',
      members: 'Profile',
    }
    let label: string
    if (!section) label = board ? `#${board}` : 'All posts'
    else if (section === 'p' && id) label = `#${id}`
    else if (section === 'agents') label = id ? (agentList?.find((a) => a.id === id)?.name ?? 'Agent') : 'Agents'
    else label = named[section] ?? ''
    if (section === 'inbox' && unreadCount) label = `(${unreadCount}) ${label}`
    setTitle(label, team.name)
  }, [team, slug, location.pathname, location.search, unreadCount, agentList])

  const base = `/${slug}`

  // Keyboard: c = new post (when not typing), in the topic you're looking at.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented || e.repeat) return
      const el = e.target as HTMLElement | null
      if (el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName))) return
      if (e.key === 'c' || e.key === 'C') {
        e.preventDefault()
        const onList = window.location.pathname.replace(/\/$/, '') === base
        const board = onList ? new URLSearchParams(window.location.search).get('board') : null
        navigate(`${base}/new-post${board ? `?board=${encodeURIComponent(board)}` : ''}`)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [navigate, base])

  const ctx = useMemo<ShellCtx | null>(() => {
    if (!me || !team) return null
    return {
      me,
      teamId,
      slug: team.slug,
      base: `/${team.slug}`,
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

  if (reserved) return <Navigate to="/" replace />
  if (meError && !me) {
    return (
      <div className="app-loading">
        <p className="muted">Could not reach the server</p>
        <button className="btn btn-sm" onClick={() => void refreshMe()}>
          Retry
        </button>
      </div>
    )
  }
  if (!me) return <BootLoader />
  if (!team) return missing === slug ? <NotMember slug={slug} me={me} refreshMe={refreshMe} /> : <BootLoader />

  return (
    <ShellContext.Provider value={ctx}>
      <TopProgress />
      <WindowDrop />
      <div className={`app ${navOpen ? 'nav-open' : ''}`}>
        <Sidebar ctx={ctx!} onClose={() => setNavOpen(false)} />
        <div className="nav-scrim" onClick={() => setNavOpen(false)} aria-hidden="true" />
        <main className="app-main">
          <RouteView key={location.pathname}>
            <Outlet />
          </RouteView>
        </main>
      </div>
    </ShellContext.Provider>
  )
}

/** One sliding highlight for the active row and a softer one that follows the pointer. */
function useGlide(ref: React.RefObject<HTMLElement | null>, deps: unknown[]) {
  const [on, setOn] = useState(false)

  useLayoutEffect(() => {
    const root = ref.current
    if (!root) return
    const active = root.querySelector<HTMLElement>('.sb-glide-active')
    const ghost = root.querySelector<HTMLElement>('.sb-glide-hover')
    if (!active || !ghost) return

    const place = (el: HTMLElement, target: HTMLElement | null, animate: boolean) => {
      if (!target || target.offsetParent === null) {
        el.style.opacity = '0'
        return
      }
      const r = root.getBoundingClientRect()
      const t = target.getBoundingClientRect()
      const x = t.left - r.left + root.scrollLeft
      const y = t.top - r.top + root.scrollTop
      const wasHidden = el.style.opacity !== '1'
      el.style.transition = animate && !wasHidden && !reducedMotion() ? '' : 'none'
      el.style.transform = `translate(${x}px, ${y}px)`
      el.style.width = `${t.width}px`
      el.style.height = `${t.height}px`
      el.style.opacity = '1'
      if (el.style.transition === 'none') {
        void el.offsetWidth // commit the jump before re-enabling transitions
        el.style.transition = ''
      }
    }

    const placeActive = (animate: boolean) => place(active, root.querySelector<HTMLElement>('.sb-link.active'), animate)
    placeActive(true)
    setOn(true)

    const onOver = (e: PointerEvent) => {
      const link = (e.target as HTMLElement).closest<HTMLElement>('.sb-link')
      if (link && root.contains(link)) place(ghost, link, true)
      else ghost.style.opacity = '0'
    }
    const onLeave = () => {
      ghost.style.opacity = '0'
    }
    const ro = new ResizeObserver(() => placeActive(false))
    ro.observe(root)
    root.addEventListener('pointerover', onOver)
    root.addEventListener('pointerleave', onLeave)
    return () => {
      ro.disconnect()
      root.removeEventListener('pointerover', onOver)
      root.removeEventListener('pointerleave', onLeave)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps)

  return on
}

function Sidebar({ ctx, onClose }: { ctx: ShellCtx; onClose: () => void }) {
  const { me, team, base, boards, agents, unread } = ctx
  const navigate = useNavigate()
  const location = useLocation()
  const [params] = useSearchParams()
  const [menu, setMenu] = useState(false)
  const [joining, setJoining] = useState(false)
  const [adding, setAdding] = useState(false)
  const [boardKey, setBoardKey] = useState('')
  const [boardError, setBoardError] = useState('')
  const menuRef = useRef<HTMLDivElement>(null)
  const sideRef = useRef<HTMLElement>(null)

  const onList = location.pathname === base || location.pathname === `${base}/`
  const activeBoard = onList ? params.get('board') : null
  const openTotal = boards.reduce((n, b) => n + b.open_count, 0)
  const activeAgents = agents.filter((a) => a.active).length
  const isNewTopic = useArrivals(boards.length ? boards.map((b) => b.id) : undefined)

  const glide = useGlide(sideRef, [location.pathname, location.search, boards.length, adding, team.id])

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
      const b = await api.createBoard(ctx.teamId, key)
      setBoardKey('')
      setAdding(false)
      await ctx.refreshBoards()
      navigate(`${base}?board=${encodeURIComponent(b.key)}`)
    } catch (err) {
      setBoardError(err instanceof Error ? err.message : 'Could not create topic')
    }
  }

  return (
    <aside className={`sidebar ${glide ? 'glide' : ''}`} aria-label="Navigation" ref={sideRef}>
      <span className="sb-glide-active" aria-hidden="true" />
      <span className="sb-glide-hover" aria-hidden="true" />
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
                  className="menu-item menu-org"
                  onClick={() => {
                    setMenu(false)
                    navigate(`/${t.slug}`)
                  }}
                >
                  <span className="menu-org-text">
                    <span className="menu-label">{t.name}</span>
                    <span className="menu-slug mono">/{t.slug}</span>
                  </span>
                  {t.id === team.id ? <Icon name="check" size={14} /> : <span className="menu-check-space" />}
                </button>
              ))}
              <div className="menu-sep" />
              <button
                role="menuitem"
                className="menu-item"
                onClick={() => {
                  setMenu(false)
                  navigate('/new')
                }}
              >
                <span className="menu-label">Create org</span>
                <Icon name="plus" size={14} />
              </button>
              {joining ? (
                <div className="menu-join">
                  <JoinForm refreshMe={ctx.refreshMe} compact autoFocus onDone={() => setMenu(false)} />
                </div>
              ) : (
                <button role="menuitem" className="menu-item" onClick={() => setJoining(true)}>
                  <span className="menu-label">Join an org</span>
                  <Icon name="logIn" size={14} />
                </button>
              )}
              <div className="menu-sep" />
              <div className="menu-meta">{me.user.email}</div>
              <button role="menuitem" className="menu-item" onClick={signOut}>
                <span className="menu-label">Sign out</span>
                <Icon name="logOut" size={14} />
              </button>
            </div>
          ) : null}
        </div>
        <Link className="btn btn-ghost btn-sm btn-icon compose" to={`${base}/new-post`} title="New post (C)" aria-label="New post">
          <Icon name="pencil" />
        </Link>
        <button className="btn btn-ghost btn-sm btn-icon nav-close" onClick={onClose} aria-label="Close menu">
          <Icon name="x" />
        </button>
      </div>

      <nav className="sb-nav">
        <SideLink to={base} icon="list" label="All posts" count={openTotal} active={onList && !activeBoard} />
        <SideLink to={`${base}/inbox`} icon="inbox" label="Inbox" count={unread} highlight={unread > 0} />
        <SideLink to={`${base}/agents`} icon="terminal" label="Agents" count={activeAgents} active={location.pathname.startsWith(`${base}/agents`)} />
        <SideLink to={`${base}/claims`} icon="lock" label="Claims" />
        <SideLink to={`${base}/activity`} icon="activity" label="Activity" />
      </nav>

      <div className="sb-section">
        <div className="sb-label">
          <span>Topics</span>
          <button
            className="btn btn-ghost btn-xs btn-icon sb-add-btn"
            onClick={() => setAdding((a) => !a)}
            aria-label="New topic"
            aria-expanded={adding}
            title="New topic"
          >
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
              fresh={isNewTopic(b.id)}
            />
          ))}
        </nav>
      </div>

      <div className="sb-bottom">
        <nav className="sb-nav">
          <SideLink to={`${base}/connect`} icon="plug" label="Connect" />
          <SideLink
            to={`${base}/team`}
            icon="users"
            label="Team"
            countText={`${team.member_count}/${team.max_members}`}
            active={
              location.pathname === `${base}/team` ||
              (location.pathname.startsWith(`${base}/members/`) && location.pathname !== `${base}/members/${me.user.id}`)
            }
          />
        </nav>
        <nav className="sb-nav sb-me-nav">
          <TNavLink
            to={`${base}/members/${me.user.id}`}
            end
            title={me.user.email}
            className={({ isActive }) => `sb-link sb-me ${isActive ? 'active' : ''}`}
          >
            <span className="sb-avatar mono" aria-hidden="true">
              {me.user.handle.slice(0, 1)}
            </span>
            <span className="sb-link-label mono">{me.user.handle}</span>
          </TNavLink>
        </nav>
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
  fresh?: boolean
}) {
  const { to, icon, label, count, countText, active, highlight, title, fresh } = props
  const text = countText ?? (count ? String(count) : '')
  return (
    <TNavLink
      to={to}
      end
      title={title}
      className={({ isActive }) => `sb-link ${(active ?? isActive) ? 'active' : ''} ${fresh ? 'arrive' : ''}`}
    >
      <Icon name={icon} />
      <span className="sb-link-label">{label}</span>
      {text ? <Count value={text} className={`sb-count ${highlight ? 'hl' : ''}`} /> : null}
    </TNavLink>
  )
}
