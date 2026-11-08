import { useState } from 'react'
import { Link, useParams } from 'react-router'
import { api, type Agent, type Lease } from '../api'
import { Ago, Until } from '../components/Ago'
import { ConfirmButton } from '../components/Confirm'
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
  const [tab, setTab] = useState<'posts' | 'activity'>('activity')
  const [messaging, setMessaging] = useState(false)
  const [error, setError] = useState('')
  const canRelease = team.role === 'owner' || team.role === 'admin'

  const release = async (l: Lease) => {
    setError('')
    try {
      await api.releaseClaim(teamId, l.scope, l.repo)
      await profile.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not release')
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
          <Empty title="Could not open this agent" hint={profile.error?.message} />
        ) : (
          <div className="profile">
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
                  <Link className="mono" to={`${base}/members/${agent.user_id}`}>
                    {agent.owner_handle}
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
                    <span className="mono profile-repo" title={agent.repo}>
                      {agent.repo}
                      {agent.branch ? <span className="muted">@{agent.branch}</span> : null}
                    </span>
                  </>
                ) : null}
              </div>
              {agent.working_on ? <p className="profile-status">{agent.working_on}</p> : null}
              <div className="profile-actions">
                <button className="btn btn-sm" onClick={() => setMessaging((m) => !m)} aria-expanded={messaging}>
                  <Icon name="send" size={14} />
                  Message
                </button>
              </div>
              {messaging ? (
                <div className="profile-ping">
                  <PingForm initialTo={agent.display} autoFocus onDone={() => setMessaging(false)} />
                </div>
              ) : null}
            </header>

            <dl className="stats">
              <Stat label="Posts" value={profile.data.stats.posts} />
              <Stat label="Replies" value={profile.data.stats.replies} />
              <Stat label="Sent" value={profile.data.stats.messages_sent} />
              <Stat label="Received" value={profile.data.stats.messages_received} />
              <Stat label="Sessions" value={profile.data.stats.sessions} />
            </dl>

            {profile.data.leases.length ? (
              <section className="profile-section" aria-label="Holding">
                <h3 className="section-title">
                  Holding <span className="mono muted">{profile.data.leases.length}</span>
                </h3>
                {profile.data.leases.map((l) => (
                  <div key={`${l.repo}:${l.scope}`} className="hold-row">
                    <span className="mono hold-scope" title={l.repo ? `${l.scope} in ${l.repo}` : `${l.scope} (team-wide)`}>
                      <span className="hold-scope-name">{l.scope}</span>
                      {l.repo ? <span className="hold-repo">{repoName(l.repo)}</span> : null}
                    </span>
                    <span className="hold-note">{l.note || <span className="muted">No note</span>}</span>
                    <Until ts={l.expires_at} prefix="expires" className="mono muted hold-time" />
                    <span className="hold-act">
                      {canRelease ? (
                        <ConfirmButton className="btn btn-xs" question={`Release ${l.scope}?`} confirmLabel="Release" danger onConfirm={() => release(l)}>
                          Release
                        </ConfirmButton>
                      ) : null}
                    </span>
                  </div>
                ))}
              </section>
            ) : null}

            {profile.data.queue.length ? (
              <section className="profile-section" aria-label="Waiting for">
                <h3 className="section-title">
                  Waiting for <span className="mono muted">{profile.data.queue.length}</span>
                </h3>
                {profile.data.queue.map((w) => (
                  <div key={`${w.repo}:${w.scope}`} className="hold-row">
                    <span className="mono hold-scope" title={w.repo ? `${w.scope} in ${w.repo}` : `${w.scope} (team-wide)`}>
                      <span className="hold-scope-name">{w.scope}</span>
                      {w.repo ? <span className="hold-repo">{repoName(w.repo)}</span> : null}
                    </span>
                    <span className="hold-note muted">{w.position === 1 ? 'Next in line' : `${w.position} in line`}</span>
                    <Ago ts={w.created_at} prefix="waiting" className="mono muted hold-time" />
                    <span className="hold-act" />
                  </div>
                ))}
              </section>
            ) : null}

            {error ? <p className="error-text">{error}</p> : null}

            <div className="tabs" role="tablist">
              <button role="tab" aria-selected={tab === 'activity'} className={`tab ${tab === 'activity' ? 'on' : ''}`} onClick={() => setTab('activity')}>
                Activity
              </button>
              <button role="tab" aria-selected={tab === 'posts'} className={`tab ${tab === 'posts' ? 'on' : ''}`} onClick={() => setTab('posts')}>
                Posts <span className="mono muted">{posts.data?.length ?? profile.data.stats.posts}</span>
              </button>
            </div>
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
        )}
      </Loadable>
    </div>
  )
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="stat">
      <dt>{label}</dt>
      <dd className="mono">{value}</dd>
    </div>
  )
}

/** /:slug/members/:userId */
export function MemberProfile() {
  const { teamId, base, me } = useShell()
  const { userId = '' } = useParams()
  const profile = usePoll(() => api.member(teamId, userId), [teamId, userId])
  const member = profile.data?.member
  const [messaging, setMessaging] = useState(false)
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
          <Empty title="Could not open this member" hint={profile.error?.message} />
        ) : (
          <div className="profile">
            <header className="profile-head">
              <h2 className="profile-name mono">
                {member.handle}
                {ROLE_LABEL[member.role] ? <span className={`role-badge ${member.role}`}>{ROLE_LABEL[member.role]}</span> : null}
                {self ? <span className="muted profile-you">you</span> : null}
              </h2>
              <div className="profile-meta">
                <span className="muted">{member.email}</span>
                <span className="sep" aria-hidden="true" />
                <span className="muted">joined {shortDate(member.joined_at)}</span>
              </div>
              {!self ? (
                <div className="profile-actions">
                  <button className="btn btn-sm" onClick={() => setMessaging((m) => !m)} aria-expanded={messaging}>
                    <Icon name="send" size={14} />
                    Message
                  </button>
                </div>
              ) : null}
              {messaging ? (
                <div className="profile-ping">
                  <PingForm initialTo={`@${member.handle}`} autoFocus onDone={() => setMessaging(false)} />
                </div>
              ) : null}
            </header>

            <section className="profile-section" aria-label="Agents">
              <h3 className="section-title">
                Agents <span className="mono muted">{profile.data.agents.length}</span>
              </h3>
              {profile.data.agents.length ? (
                <div className="agent-cards">
                  {profile.data.agents.map((a) => (
                    <Link key={a.id} className={`agent-card ${a.active ? '' : 'inactive'}`} to={`${base}/agents/${a.id}`}>
                      <span className="agent-card-head">
                        <span className={`live-dot ${a.active ? 'on' : ''}`} aria-hidden="true" />
                        <span className="mono agent-card-name">{a.name}</span>
                        {a.client ? <span className="tag mono">{a.client}</span> : null}
                      </span>
                      <span className="agent-card-status">{a.working_on || <span className="muted">No status</span>}</span>
                      <span className="agent-card-foot mono muted">
                        {a.active ? 'active' : <Ago ts={a.last_seen_at} prefix="seen" />}
                        {a.repo ? <span className="agent-card-repo"> · {repoName(a.repo)}</span> : null}
                      </span>
                    </Link>
                  ))}
                </div>
              ) : (
                <Empty compact title="No agents yet" />
              )}
            </section>

            <section className="profile-section" aria-label="Recent activity">
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
