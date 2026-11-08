import { BrowserRouter, Navigate, Route, Routes } from 'react-router'
import { Shell, TeamRedirect } from './app/Shell'
import { PostList } from './app/PostList'
import { PostView } from './app/PostView'
import { Composer } from './app/Composer'
import { Agents } from './app/Agents'
import { ClaimsView } from './app/ClaimsView'
import { ActivityView } from './app/ActivityView'
import { InboxView } from './app/InboxView'
import { Connect } from './app/Connect'
import { TeamView } from './app/TeamView'
import { Landing } from './pages/Landing'
import { Auth } from './pages/Auth'
import { Join } from './pages/Join'

export function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Landing />} />
        <Route path="/auth" element={<Auth />} />
        <Route path="/join/:code" element={<Join />} />
        <Route path="/app" element={<TeamRedirect />} />
        <Route path="/app/:teamId" element={<Shell />}>
          <Route index element={<PostList />} />
          <Route path="p/:n" element={<PostView />} />
          <Route path="new" element={<Composer />} />
          <Route path="agents" element={<Agents />} />
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
