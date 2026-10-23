import { createFileRoute, Link } from '@tanstack/react-router'
import { useSuspenseQuery } from '@tanstack/react-query'
import { sessionOptions, accountSettingsOptions } from '../lib/session'
import { accountReady } from '../lib/api'
import { Composer, Timeline } from '../components/social'

export const Route = createFileRoute('/')({ component: Home })

function Home() {
  const { data: session } = useSuspenseQuery(sessionOptions)
  const { data: settings } = useSuspenseQuery(accountSettingsOptions)
  if (accountReady(session)) return <main className="page social-page">
    <h1>Home feed</h1>
    <p>Your posts and the people you follow, newest first.</p>
    <Composer />
    <Timeline path="/api/v1/feed" label="Home feed posts" />
  </main>
  return (
    <main className="page">
      <p className="eyebrow">TwitterCloneAgent</p>
      <h1>A little less noise.<br />A little more conversation.</h1>
      <p className="intro">
        A text-first social app, built with Rust and TanStack Start.
      </p>
      <section className="notice" aria-labelledby="status-heading">
        <h2 id="status-heading">The foundation is in place.</h2>
        <p>
          {settings.emailEnabled === false ? 'Read public profiles and posts, or log in to join the conversation.' : 'Read public profiles and posts, or verify your account to join the conversation.'}
        </p>
        {session ? <p><Link to="/verify-email">Verify email to use your feed and publish posts</Link></p>
          : <p><Link to="/register">Create an account</Link> · <Link to="/login">Log in</Link></p>}
      </section>
    </main>
  )
}
