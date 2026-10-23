import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery, useSuspenseQuery } from '@tanstack/react-query'
import { AccountForm, useAccount } from '../components/account-form'
import { ReadFailure } from '../components/social'
import { profileOptions, socialKey } from '../lib/social'
import { sessionOptions } from '../lib/session'
import { ApiFailure, accountReady } from '../lib/api'

export const Route = createFileRoute('/settings/profile')({ component: ProfileEditor })

function ProfileEditor() {
  const { data: session } = useSuspenseQuery(sessionOptions)
  if (!session) return <main className="page"><h1>Edit profile</h1><Link to="/login">Log in to edit your profile</Link></main>
  if (!accountReady(session)) return <main className="page"><h1>Edit profile</h1><Link to="/verify-email">Verify email before editing your profile</Link></main>
  return <Editor handle={session.handle} />
}

function Editor({ handle }: { handle: string }) {
  const { api } = useAccount()
  const { queryClient } = Route.useRouteContext()
  const profile = useQuery(profileOptions(api, handle))
  if (profile.isPending) return <main className="page"><p role="status">Loading profile…</p></main>
  if (!profile.data) return <main className="page"><ReadFailure error={profile.error} retry={() => { void profile.refetch() }} /></main>
  return <main className="page account-page">
    <h1>Edit profile</h1>
    <p>Your handle @{handle} cannot be changed.</p>
    <AccountForm label="Save profile" success="Profile saved." submit={async (form) => {
      const displayName = String(form.get('displayName') ?? '')
      const biography = String(form.get('biography') ?? '')
      if (Array.from(displayName).length < 1 || Array.from(displayName).length > 50 || Array.from(biography).length > 160) {
        throw new ApiFailure(422, 'invalid_profile', 'Display names require 1–50 Unicode code points and biographies at most 160.')
      }
      await api.mutate('/api/v1/me', 'PATCH', { displayName, biography })
      await queryClient.invalidateQueries({ queryKey: socialKey })
    }}>
      <div className="field"><label htmlFor="display-name">Display name</label>
        <input id="display-name" name="displayName" required defaultValue={profile.data.displayName} aria-describedby="profile-limits" />
      </div>
      <div className="field"><label htmlFor="biography">Biography</label>
        <textarea id="biography" name="biography" rows={4} defaultValue={profile.data.biography} aria-describedby="profile-limits" />
      </div>
      <p id="profile-limits">Display name: 1–50 Unicode code points. Biography: up to 160. Plain text only.</p>
    </AccountForm>
    <Link to="/profiles/$handle" params={{ handle }}>View your profile</Link>
  </main>
}
