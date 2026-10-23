import { Link, useRouter } from '@tanstack/react-router'
import { useInfiniteQuery, useSuspenseQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { ApiFailure, accountReady, type Post, type Profile } from '../lib/api'
import { socialError, socialKey, timelineOptions, uniquePosts } from '../lib/social'
import { sessionOptions } from '../lib/session'
import { codePoints, validPostBody } from '../lib/text'
import { PlainText } from './plain-text'

export function ReadFailure({ error, retry }: { error: unknown; retry: () => void }) {
  if (error instanceof ApiFailure && error.status === 404) {
    return <p role="status">This profile or post was not found.</p>
  }
  return <div><p role="alert">{socialError(error)}</p>
    <button type="button" onClick={retry}>Try again</button></div>
}

export function PostCard({ post }: { post: Post }) {
  const { data: session } = useSuspenseQuery(sessionOptions)
  const { api, queryClient } = useRouter().options.context!
  const [confirmDelete, setConfirmDelete] = useState(false)
  return <article className="post-card" aria-label={`Post by @${post.authorHandle}`} data-post-id={post.id}>
    <p><Link to="/profiles/$handle" params={{ handle: post.authorHandle }}>
      <PlainText text={post.authorDisplayName} /> @{post.authorHandle}
    </Link></p>
    <p><PlainText text={post.body} /></p>
    <p><Link to="/posts/$id" params={{ id: post.id }}>
      <time dateTime={post.createdAt}>{new Date(post.createdAt).toISOString().replace('T', ' ').replace('.000Z', ' UTC')}</time>
    </Link> · {post.likeCount} {post.likeCount === 1 ? 'like' : 'likes'}</p>
    {accountReady(session) && <div className="actions">
      <SocialAction label={post.viewerLikes ? 'Unlike' : 'Like'} pressed={post.viewerLikes}
        action={async () => {
          await api.mutate(`/api/v1/posts/${post.id}/like`, post.viewerLikes ? 'DELETE' : 'PUT')
          await queryClient.invalidateQueries({ queryKey: socialKey })
        }} />
      {session.id === post.authorId && (confirmDelete
        ? <><SocialAction label="Confirm delete post" action={async () => {
          await api.mutate(`/api/v1/posts/${post.id}`, 'DELETE')
          await queryClient.invalidateQueries({ queryKey: socialKey })
        }} /><button type="button" onClick={() => setConfirmDelete(false)}>Keep post</button></>
        : <button type="button" onClick={() => setConfirmDelete(true)}>Delete post</button>)}
    </div>}
    {session && !accountReady(session) && <p><Link to="/verify-email">Verify email to interact</Link></p>}
  </article>
}

// Server-confirmed changes only. Each control has a synchronous submission lock
// in addition to disabled UI; errors never optimistically change counts/state.
export function SocialAction({ label, pressed, action }: {
  label: string; pressed?: boolean; action: () => Promise<void>
}) {
  const [hydrated, setHydrated] = useState(false)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState('')
  const lock = useRef(false)
  useEffect(() => { setHydrated(true) }, [])
  return <div>
    <button type="button" aria-pressed={pressed} disabled={!hydrated || pending}
      aria-busy={pending} onClick={async () => {
        if (lock.current) return
        lock.current = true
        setPending(true); setError('')
        try { await action() } catch (error) { setError(socialError(error)) }
        finally { lock.current = false; setPending(false) }
      }}>{pending ? `${label}…` : label}</button>
    {error && <p role="alert" className="form-error">{error}</p>}
  </div>
}

export function FollowControl({ profile }: { profile: Profile }) {
  const { api, queryClient } = useRouter().options.context!
  const { data: session } = useSuspenseQuery(sessionOptions)
  if (!accountReady(session) || session.id === profile.id) return null
  return <SocialAction label={profile.viewerFollows ? 'Unfollow' : 'Follow'}
    pressed={profile.viewerFollows} action={async () => {
      await api.mutate(`/api/v1/me/following/${profile.id}`, profile.viewerFollows ? 'DELETE' : 'PUT')
      await queryClient.invalidateQueries({ queryKey: socialKey })
    }} />
}

export function Composer() {
  const { api, queryClient } = useRouter().options.context!
  const [body, setBody] = useState('')
  const [pending, setPending] = useState(false)
  const [hydrated, setHydrated] = useState(false)
  const [error, setError] = useState('')
  const [message, setMessage] = useState('')
  const lock = useRef(false)
  const count = codePoints(body)
  const valid = validPostBody(body)
  useEffect(() => { setHydrated(true) }, [])
  return <section aria-labelledby="composer-heading">
    <h2 id="composer-heading">Write a post</h2>
    <form method="post" aria-busy={pending} onSubmit={async (event) => {
      event.preventDefault()
      if (lock.current || !valid) return
      lock.current = true; setPending(true); setError(''); setMessage('')
      try {
        await api.mutate('/api/v1/posts', 'POST', { body })
        setBody(''); setMessage('Post published.')
        await queryClient.invalidateQueries({ queryKey: socialKey })
      } catch (error) {
        setError(socialError(error) + ' Your text is kept. After a connection failure, check your feed before submitting again.')
      } finally { lock.current = false; setPending(false) }
    }}>
      <div className="field"><label htmlFor="post-body">Post text</label>
        <textarea id="post-body" name="body" value={body} rows={4} disabled={pending}
          aria-describedby="post-count post-policy" onChange={(event) => setBody(event.target.value)} />
      </div>
      <p id="post-count" aria-live="polite">{count}/280 Unicode code points</p>
      <p id="post-policy">Plain text, with at least one non-whitespace character.</p>
      <button type="submit" disabled={!hydrated || pending || !valid}>{pending ? 'Publishing…' : 'Publish post'}</button>
      {error && <p role="alert" className="form-error">{error}</p>}
      {message && <p role="status">{message}</p>}
    </form>
  </section>
}

export function Timeline({ path, label }: { path: `/api/v1/${string}`; label: string }) {
  const { api } = useRouter().options.context!
  const timeline = useInfiniteQuery(timelineOptions(api, path))
  const posts = uniquePosts(timeline.data?.pages ?? [])
  return <section aria-label={label} aria-busy={timeline.isFetching}>
    {timeline.isPending && <p role="status">Loading posts…</p>}
    {posts.map((post) => <PostCard key={post.id} post={post} />)}
    {timeline.isError && <ReadFailure error={timeline.error}
      retry={() => { void (timeline.isFetchNextPageError ? timeline.fetchNextPage() : timeline.refetch()) }} />}
    {!timeline.isPending && !timeline.isError && posts.length === 0 && <p role="status">No posts yet.</p>}
    {timeline.hasNextPage
      ? <button type="button" disabled={timeline.isFetching} onClick={() => { void timeline.fetchNextPage() }}>
        {timeline.isFetchingNextPage ? 'Loading more…' : 'Load more posts'}
      </button>
      : !timeline.isPending && !timeline.isError && posts.length > 0 && <p role="status">You’re all caught up.</p>}
  </section>
}
