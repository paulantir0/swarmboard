import { Link } from 'react-router'
import type { Me } from '../api'
import { Wordmark } from '../components/Brand'
import { Icon } from '../components/Icon'
import { JoinForm } from './JoinForm'
import { HOST } from '../lib/org'

/** /<slug> you can't see: paste an invite, or go to one of yours. */
export function NotMember({ slug, me, refreshMe }: { slug: string; me: Me; refreshMe: () => Promise<void> }) {
  return (
    <main className="solo">
      <div className="solo-card">
        <Link to="/" className="solo-brand" aria-label="swarmboard">
          <Wordmark />
        </Link>
        <h1 className="solo-title">You’re not a member of this org</h1>
        <p className="solo-body">
          <code className="mono">
            {HOST}/{slug}
          </code>{' '}
          doesn’t exist, or you haven’t been invited. Paste an invite link to join it.
        </p>
        <div className="solo-section solo-section-first">
          <JoinForm refreshMe={refreshMe} autoFocus label="Invite link" />
        </div>
        <div className="solo-section">
          <h2 className="solo-section-title">{me.teams.length ? 'Your orgs' : 'Start your own'}</h2>
          {me.teams.length ? (
            <ul className="solo-orgs">
              {me.teams.map((t) => (
                <li key={t.id}>
                  <Link className="solo-org" to={`/${t.slug}`}>
                    <span className="solo-org-name">{t.name}</span>
                    <span className="mono muted">/{t.slug}</span>
                    <Icon name="chevronRight" size={14} />
                  </Link>
                </li>
              ))}
            </ul>
          ) : null}
          <div className="solo-actions">
            <Link className="btn" to="/new">
              <Icon name="plus" size={14} />
              Create an org
            </Link>
          </div>
        </div>
        <p className="solo-foot">
          Signed in as <span className="solo-email">{me.user.email}</span>
        </p>
      </div>
    </main>
  )
}
