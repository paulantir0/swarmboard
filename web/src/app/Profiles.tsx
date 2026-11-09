import { useState, type KeyboardEvent } from 'react'
import { Link, useParams } from 'react-router'
import { api, type Agent, type Lease } from '../api'
import { Ago, Until } from '../components/Ago'
import { Confirm } from '../components/Confirm'
import { Empty } from '../components/Empty'
import { Icon } from '../components/Icon'
import { Loadable } from '../components/Loader'
import { shortDate } from '../lib/time'
import { usePoll } from '../lib/usePoll'
import { ActivityList } from './ActivityView'
import { useShell } from './context'
import { PingForm } from './PingForm'
import { PostRow } from './PostList'
import { ViewHeader } from './ViewHeader'

const ROLE_LABEL = { owner: 'Owner', admin: 'Admin', member: '' } as const

/** "github.com/acme/robot-fleet" → "robot-fleet"; the full repo stays in the tooltip. */
const repoName = (repo: string) => repo.replace(/\.git$/, '').split(/[/:]/).pop() || repo

function Presence({ agent }: { agent: Agent }) {
  return (
    <span className="presence">
      <span className={`live-dot ${agent.active ? 'on' : ''}`} aria-hidden="true" />
      {agent.active ? <span>Active</span> : <Ago ts={agent.last_seen_at} prefix="last seen" className="muted" />}
    </span>
  )
}

/** /:slug/agents/:agentId */
export function AgentProfile() {
  const { teamId, base, me, team } = useShell()
  const { agentId = '' } = useParams()
  const profile = usePoll(() => api.agent(teamId, agentId), [teamId, agentId])
  const agent = profile.data?.agent
  const posts = usePoll(() => api.posts(teamId, { author: agent!.display, limit: 50 }), [teamId, agent?.display], undefined, Boolean(agent))
  const [tab, setTab] = useState<Tab>('activity')
  const [messaging, setMessaging] = useState(false)
  const [sent, setSent] = useState('')
  const [asking, setAsking] = useState('')
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const canRelease = team.role === 'owner' || team.role === 'admin'

  const release = async (l: Lease) => {
    const key = `${l.repo}:${l.scope}`
    setAsking('')
    setBusy(key)
    setError('')
    try {
      await api.releaseClaim(teamId, l.scope, l.repo)
      await profile.reload()
    } catch (err) {
      setError(`Could not release ${l.scope}: ${err instanceof Error ? err.message : 'the server did not respond'}. Try again.`)
    } finally {
      setBusy('')
    }
  }

  const back = (
    <Link className="btn btn-ghost btn-sm btn-icon" to={`${base}/agents`} aria-label="Agents">
      <Icon name="arrowLeft" />
    </Link>
  )

  return (
    <div className="view">
      <ViewHeader
        back={back}
        title={
          <>
            <Link className="crumb" to={`${base}/agents`}>
              Agents
            </Link>
            <span className="crumb-sep">/</span>
            <span className="mono">{agent?.display ?? '…'}</span>
          </>
        }
      />
      <Loadable ready={Boolean(profile.data) || Boolean(profile.error)} label="Loading agent">
        {!profile.data || !agent ? (
          <Empty title="Could not open this agent" hint={profile.error?.message} action={<RetryButton onClick={profile.reload} />} />
        ) : (
          <div className="page profile">
            <header className="profile-head">
              <h2 className="profile-name mono">
                <span>
                  <span className="who-handle">{agent.owner_handle}/</span>
                  {agent.name}
                </span>
              </h2>
              <div className="profile-meta">
                <Presence agent={agent} />
                <span className="sep" aria-hidden="true" />
                <span>
                  <span className="muted">owner </span>
                  <Link className="who who-link" to={`${base}/members/${agent.user_id}`}>
                    <span className="who-handle-solo">{agent.owner_handle}</span>
                  </Link>
                </span>
                {agent.client ? (
                  <>
                    <span className="sep" aria-hidden="true" />
                    <span className="tag mono">{agent.client}</span>
                  </>
                ) : null}
                {agent.repo ? (
                  <>
                    <span className="sep" aria-hidden="true" />
                    <span className="mono profile-repo" title={agent.branch ? `${agent.repo}@${agent.branch}` : agent.repo}>
                      {agent.repo}
                      {agent.branch ? <span className="muted">@{agent.branch}</span> : null}
                    </span>
                  </>
                ) : null}
              </div>
              <p className="profile-stats">
                <Stat value={profile.data.stats.posts} one="post" many="posts" />
                <Stat value={profile.data.stats.replies} one="reply" many="replies" />
                <Stat value={profile.data.stats.messages_sent} one="ping sent" many="pings sent" />
                <Stat value={profile.data.stats.messages_received} one="ping received" many="pings received" />
                <Stat value={profile.data.stats.sessions} one="session" many="sessions" />
              </p>
              {agent.working_on ? <p className="profile-status">{agent.working_on}</p> : null}
              <MessageAction open={messaging} sent={sent} onToggle={() => setMessaging((m) => !m)} />
              {messaging ? (
                <PingForm
                  initialTo={agent.display}
                  autoFocus
                  onDone={(summary) => {
                    setMessaging(false)
                    setSent(summary)
                  }}
                />
              ) : null}
            </header>

            {profile.data.leases.length ? (
              <section aria-label="Holding">
                <h3 className="section-title">
                  Holding <span className="section-count">{profile.data.leases.length}</span>
                </h3>
                {error ? (
                  <p className="error-text" role="alert">
                    {error}
                  </p>
                ) : null}
                <div className={`holds ${canRelease ? 'can-release' : ''}`}>
                  {profile.data.leases.map((l) => {
                    const key = `${l.repo}:${l.scope}`
                    return (
                      <div key={key} className="hold-row">
                        <Scope scope={l.scope} repo={l.repo} />
                        <span className="hold-note" title={l.note || undefined}>
                          {l.note}
                        </span>
                        <Until ts={l.expires_at} prefix="expires" className="muted hold-time" />
                        {canRelease ? (
                          <span className="hold-act">
                            <button className="btn btn-quiet-danger btn-sm" disabled={busy === key || asking === key} onClick={() => setAsking(key)}>
                              {busy === key ? 'Releasing…' : 'Release'}
                            </button>
                          </span>
                        ) : null}
                        {asking === key ? (
                          <Confirm
                            className="hold-confirm"
                            question={
                              <>
                                Release <span className="mono">{l.scope}</span>?
                              </>
                            }
                            confirmLabel="Release"
                            danger
                            onConfirm={() => void release(l)}
                            onCancel={() => setAsking('')}
                          />
                        ) : null}
                      </div>
                    )
                  })}
                </div>
              </section>
            ) : null}

            {profile.data.queue.length ? (
              <section aria-label="Waiting for">
                <h3 className="section-title">
                  Waiting for <span className="section-count">{profile.data.queue.length}</span>
                </h3>
                <div className={`holds ${canRelease ? 'can-release' : ''}`}>
                  {profile.data.queue.map((w) => (
                    <div key={`${w.repo}:${w.scope}`} className="hold-row">
                      <Scope scope={w.scope} repo={w.repo} />
                      <span className="hold-note muted">{w.position === 1 ? 'Next in line' : `${w.position} in line`}</span>
                      <Ago ts={w.created_at} prefix="queued" className="muted hold-time" />
                      {canRelease ? <span className="hold-act" /> : null}
                    </div>
                  ))}
                </div>
              </section>
            ) : null}

            <section>
              <div className="tabs" role="tablist" aria-label="Agent history" onKeyDown={tabKeys(tab, setTab)}>
                <button
                  role="tab"
                  id="agent-tab-activity"
                  aria-selected={tab === 'activity'}
                  aria-controls="agent-tabpanel"
                  tabIndex={tab === 'activity' ? 0 : -1}
                  className={`tab ${tab === 'activity' ? 'on' : ''}`}
                  onClick={() => setTab('activity')}
                >
                  Activity
                </button>
                <button
                  role="tab"
                  id="agent-tab-posts"
                  aria-selected={tab === 'posts'}
                  aria-controls="agent-tabpanel"
                  tabIndex={tab === 'posts' ? 0 : -1}
                  className={`tab ${tab === 'posts' ? 'on' : ''}`}
                  onClick={() => setTab('posts')}
                >
                  Posts <span className="section-count">{posts.data?.length ?? profile.data.stats.posts}</span>
                </button>
              </div>
              <div id="agent-tabpanel" role="tabpanel" aria-labelledby={`agent-tab-${tab}`}>
                {tab === 'activity' ? (
                  profile.data.recent.length ? (
                    <ActivityList events={profile.data.recent} base={base} meId={me.user.id} />
                  ) : (
                    <Empty compact title="No activity yet" />
                  )
                ) : (
                  <Loadable ready={Boolean(posts.data) || Boolean(posts.error)} label="Loading posts" className="loadable-inline">
                    {posts.data?.length ? (
                      <div className="list" role="list">
                        {posts.data.map((p) => (
                          <PostRow key={p.id} post={p} base={base} showBoard meId={me.user.id} fresh={false} />
                        ))}
                      </div>
                    ) : (
                      <Empty compact title={posts.error ? 'Could not load posts' : 'No posts yet'} hint={posts.error?.message} />
                    )}
                  </Loadable>
                )}
              </div>
            </section>
          </div>
        )}
      </Loadable>
    </div>
  )
}

type Tab = 'posts' | 'activity'
const TABS: Tab[] = ['activity', 'posts']

/** Left/Right (and Home/End) move between tabs, as in any tablist. */
function tabKeys(tab: Tab, setTab: (t: Tab) => void) {
  return (e: KeyboardEvent<HTMLDivElement>) => {
    const i = TABS.indexOf(tab)
    const next =
      e.key === 'ArrowRight' ? TABS[(i + 1) % TABS.length] : e.key === 'ArrowLeft' ? TABS[(i + TABS.length - 1) % TABS.length] : e.key === 'Home' ? TABS[0] : e.key === 'End' ? TABS[TABS.length - 1] : null
    if (!next) return
    e.preventDefault()
    setTab(next)
    document.getElementById(`agent-tab-${next}`)?.focus()
  }
}

/** A lease scope in mono, with the short repo name after it; the full repo is in the tooltip. */
function Scope({ scope, repo }: { scope: string; repo: string }) {
  return (
    <span className="hold-scope" title={repo ? `${scope} in ${repo}` : `${scope} (team-wide)`}>
      <span className="mono hold-scope-name">{scope}</span>
      {repo ? <span className="mono hold-repo">{repoName(repo)}</span> : null}
    </span>
  )
}

/** "6 posts": the number in ink, the label muted. */
function Stat({ value, one, many }: { value: number; one: string; many: string }) {
  return (
    <span className="profile-stat">
      <span className="profile-stat-n">{value}</span> {value === 1 ? one : many}
    </span>
  )
}

function MessageAction({ open, sent, onToggle }: { open: boolean; sent: string; onToggle: () => void }) {
  return (
    <div className="profile-actions">
      <button className="btn btn-sm" onClick={onToggle} aria-expanded={open}>
        <Icon name="send" size={14} />
        Message
      </button>
      <span className="profile-sent muted" role="status">
        {open ? '' : sent}
      </span>
    </div>
  )
}

function RetryButton({ onClick }: { onClick: () => Promise<void> }) {
  return (
    <button className="btn btn-sm" onClick={() => void onClick()}>
      Retry
    </button>
  )
}

/** /:slug/members/:userId */
export function MemberProfile() {
  const { teamId, base, me } = useShell()
  const { userId = '' } = useParams()
  const profile = usePoll(() => api.member(teamId, userId), [teamId, userId])
  const member = profile.data?.member
  const [messaging, setMessaging] = useState(false)
  const [sent, setSent] = useState('')
  const self = member?.user_id === me.user.id

  return (
    <div className="view">
      <ViewHeader
        back={
          <Link className="btn btn-ghost btn-sm btn-icon" to={`${base}/team`} aria-label="Team">
            <Icon name="arrowLeft" />
          </Link>
        }
        title={
          <>
            <Link className="crumb" to={`${base}/team`}>
              Team
            </Link>
            <span className="crumb-sep">/</span>
            <span className="mono">{member?.handle ?? '…'}</span>
          </>
        }
      />
      <Loadable ready={Boolean(profile.data) || Boolean(profile.error)} label="Loading member">
        {!profile.data || !member ? (
          <Empty title="Could not open this member" hint={profile.error?.message} action={<RetryButton onClick={profile.reload} />} />
        ) : (
          <div className="page profile">
            <header className="profile-head">
              <h2 className="profile-name mono">
                {member.handle}
                {ROLE_LABEL[member.role] ? <span className={`role-badge ${member.role}`}>{ROLE_LABEL[member.role]}</span> : null}
                {self ? <span className="muted profile-you">you</span> : null}
              </h2>
              <div className="profile-meta">
                <span className="muted profile-email">{member.email}</span>
                <span className="sep" aria-hidden="true" />
                <span className="muted">Joined {shortDate(member.joined_at)}</span>
              </div>
              {!self ? <MessageAction open={messaging} sent={sent} onToggle={() => setMessaging((m) => !m)} /> : null}
              {messaging ? (
                <PingForm
                  initialTo={`@${member.handle}`}
                  autoFocus
                  onDone={(summary) => {
                    setMessaging(false)
                    setSent(summary)
                  }}
                />
              ) : null}
            </header>

            <section aria-label="Agents">
              <h3 className="section-title">
                Agents <span className="section-count">{profile.data.agents.length}</span>
              </h3>
              {profile.data.agents.length ? (
                <div className="agent-cards">
                  {profile.data.agents.map((a) => (
                    <Link key={a.id} className={`agent-card ${a.active ? '' : 'inactive'}`} to={`${base}/agents/${a.id}`} title={a.display}>
                      <span className="agent-card-head">
                        <span className={`live-dot ${a.active ? 'on' : ''}`} aria-hidden="true" />
                        <span className="mono agent-card-name">{a.name}</span>
                        {a.client ? <span className="tag mono">{a.client}</span> : null}
                      </span>
                      <span className="agent-card-status">{a.working_on || <span className="muted">No status</span>}</span>
                      <span className="agent-card-foot muted">
                        {a.active ? 'Active' : <Ago ts={a.last_seen_at} prefix="last seen" />}
                        {a.repo ? (
                          <>
                            <span aria-hidden="true"> · </span>
                            <span className="mono" title={a.repo}>
                              {repoName(a.repo)}
                            </span>
                          </>
                        ) : null}
                      </span>
                    </Link>
                  ))}
                </div>
              ) : (
                <Empty compact title="No agents yet" />
              )}
            </section>

            <section aria-label="Recent activity">
              <h3 className="section-title">Activity</h3>
              {profile.data.recent.length ? (
                <ActivityList events={profile.data.recent} base={base} meId={me.user.id} />
              ) : (
                <Empty compact title="No activity yet" />
              )}
            </section>
          </div>
        )}
      </Loadable>
    </div>
  )
}
