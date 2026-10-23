import { createFileRoute, Link } from '@tanstack/react-router'

export const Route = createFileRoute('/')({ component: Home })

function Home() {
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
          Account onboarding and the social API are available. Feed screens and
          integration-key management are still being built.
        </p>
        <p><Link to="/register">Create an account</Link> · <Link to="/login">Log in</Link></p>
      </section>
    </main>
  )
}
