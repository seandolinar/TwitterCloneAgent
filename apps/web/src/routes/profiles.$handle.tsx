import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery, useSuspenseQuery } from '@tanstack/react-query'
import { PlainText } from '../components/plain-text'
import { FollowControl, ReadFailure, Timeline } from '../components/social'
import { profileOptions } from '../lib/social'
import { sessionOptions } from '../lib/session'

export const Route = createFileRoute('/profiles/$handle')({
  loader: ({ context, params }) => context.queryClient.prefetchQuery(profileOptions(context.api, params.handle)),
  component: ProfilePage,
})

function ProfilePage() {
  const { handle } = Route.useParams()
  const { api } = Route.useRouteContext()
  const profile = useQuery(profileOptions(api, handle))
  const { data: session } = useSuspenseQuery(sessionOptions)
  if (profile.isPending) return <main className="page"><p role="status">Loading profile…</p></main>
  if (!profile.data || profile.isError) return <main className="page"><h1>Profile</h1>
    <ReadFailure error={profile.error} retry={() => { void profile.refetch() }} /></main>
  const person = profile.data
  return <main className="page social-page">
    <h1><PlainText text={person.displayName} /></h1>
    <p>@{person.handle}</p>
    <p><PlainText text={person.biography} /></p>
    <p>Joined <time dateTime={person.joinedAt}>{person.joinedAt.slice(0, 10)}</time></p>
    <p>{person.followerCount} followers · {person.followingCount} following · {person.postCount} posts</p>
    <FollowControl profile={person} />
    {session?.verified && session.id === person.id && <p><Link to="/settings/profile">Edit profile</Link></p>}
    {!session && <p><Link to="/login">Log in to follow and like</Link></p>}
    <h2>Posts</h2>
    <Timeline key={person.id} path={`/api/v1/profiles/${person.handle}/posts`} label="Profile posts" />
  </main>
}
