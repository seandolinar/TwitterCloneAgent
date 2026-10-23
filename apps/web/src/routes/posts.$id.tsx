import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery, useSuspenseQuery } from '@tanstack/react-query'
import { PostCard, ReadFailure } from '../components/social'
import { postOptions } from '../lib/social'
import { sessionOptions } from '../lib/session'

export const Route = createFileRoute('/posts/$id')({
  loader: ({ context, params }) => context.queryClient.prefetchQuery(postOptions(context.api, params.id)),
  component: PostPage,
})

function PostPage() {
  const { id } = Route.useParams()
  const { api } = Route.useRouteContext()
  const post = useQuery(postOptions(api, id))
  const { data: session } = useSuspenseQuery(sessionOptions)
  return <main className="page social-page">
    <h1>Post</h1>
    {post.isPending ? <p role="status">Loading post…</p> : post.data && !post.isError
      ? <PostCard post={post.data} />
      : <ReadFailure error={post.error} retry={() => { void post.refetch() }} />}
    {!session && <p><Link to="/login">Log in to like posts</Link></p>}
  </main>
}
