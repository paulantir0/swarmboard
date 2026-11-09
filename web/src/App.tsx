import { useEffect } from 'react'
import { BrowserRouter, Navigate, Route, Routes, useLocation } from 'react-router'
import { LegacyRedirect, Shell, TeamRedirect } from './app/Shell'
import { PostList } from './app/PostList'
import { PostView } from './app/PostView'
import { Composer } from './app/Composer'
import { Agents } from './app/Agents'
import { ClaimsView } from './app/ClaimsView'
import { ActivityView } from './app/ActivityView'
import { InboxView } from './app/InboxView'
import { Connect } from './app/Connect'
import { TeamView } from './app/TeamView'
import { NewOrg } from './app/NewOrg'
import { AgentProfile, MemberProfile } from './app/Profiles'
import { Landing } from './pages/Landing'
import { Auth } from './pages/Auth'
import { Join } from './pages/Join'
import { setTitle } from './lib/title'

const STATIC_TITLES: Record<string, string | null> = { '/': null, '/auth': 'Sign in', '/new': 'Create org' }

/** Tab titles for the pages outside an org; the shell titles everything under /:slug. */
function StaticTitles() {
  const { pathname } = useLocation()
  useEffect(() => {
    if (pathname in STATIC_TITLES) setTitle(STATIC_TITLES[pathname])
    else if (pathname.startsWith('/join/')) setTitle('Join')
  }, [pathname])
  return null
}

// Static paths outrank "/:slug" in React Router, and the shell bounces reserved slugs to "/".
export function App() {
  return (
    <BrowserRouter>
      <StaticTitles />
      <Routes>
        <Route path="/" element={<Landing />} />
        <Route path="/auth" element={<Auth />} />
        <Route path="/join/:code" element={<Join />} />
        <Route path="/new" element={<NewOrg />} />
        <Route path="/app" element={<TeamRedirect />} />
        <Route path="/app/:teamId/*" element={<LegacyRedirect />} />
        <Route path="/:slug" element={<Shell />}>
          <Route index element={<PostList />} />
          <Route path="p/:n" element={<PostView />} />
          <Route path="new-post" element={<Composer />} />
          <Route path="agents" element={<Agents />} />
          <Route path="agents/:agentId" element={<AgentProfile />} />
          <Route path="members/:userId" element={<MemberProfile />} />
          <Route path="claims" element={<ClaimsView />} />
          <Route path="activity" element={<ActivityView />} />
          <Route path="inbox" element={<InboxView />} />
          <Route path="connect" element={<Connect />} />
          <Route path="team" element={<TeamView />} />
          <Route path="*" element={<Navigate to="." replace />} />
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  )
}
