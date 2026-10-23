import {
  createRootRouteWithContext,
  HeadContent,
  Link,
  Outlet,
  Scripts,
  useRouter,
  type ErrorComponentProps,
} from '@tanstack/react-router'
import type { ReactNode } from 'react'
import { useSuspenseQuery } from '@tanstack/react-query'
import type { createQueryState } from '../lib/query-state'
import { sessionOptions } from '../lib/session'
import { PlainText } from '../components/plain-text'
import stylesheet from '../styles.css?url'

export const Route = createRootRouteWithContext<ReturnType<typeof createQueryState>>()({
  beforeLoad: async ({ context }) => {
    await context.queryClient.ensureQueryData(sessionOptions)
  },
  headers: () => ({ 'Cache-Control': 'private, no-store', 'Referrer-Policy': 'no-referrer' }),
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1' },
      { title: 'TwitterCloneAgent' },
    ],
    links: [{ rel: 'stylesheet', href: stylesheet }],
  }),
  component: RootLayout,
  shellComponent: RootDocument,
  errorComponent: RootError,
  notFoundComponent: () => (
    <main className="page">
      <h1>Page not found</h1>
      <a href="/">Return home</a>
    </main>
  ),
})

function RootLayout() {
  const { data: session } = useSuspenseQuery(sessionOptions)
  return <>
    <header className="site-header">
      <nav aria-label="Main navigation">
        <Link to="/">Home</Link>
        {session ? <>
          {!session.verified && <Link to="/verify-email">Verify email</Link>}
          <Link to="/logout">Log out</Link>
        </> : <>
          <Link to="/register">Create account</Link>
          <Link to="/login">Log in</Link>
        </>}
      </nav>
      <p className="session-status"><PlainText text={session ? `Signed in as @${session.handle}${session.verified ? '' : ' — email verification required'}` : 'Signed out'} /></p>
    </header>
    <Outlet />
  </>
}

function RootError({ reset }: ErrorComponentProps) {
  const router = useRouter()
  return <main className="page">
    <h1>Page unavailable</h1>
    <p role="alert">The page could not be loaded. Check your connection and try again.</p>
    <button type="button" onClick={() => { void router.invalidate(); reset() }}>Try again</button>
    <p><a href="/">Return home</a></p>
  </main>
}

function RootDocument({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <head>
        <HeadContent />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  )
}
