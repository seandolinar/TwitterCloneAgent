import { createFileRoute } from '@tanstack/react-router'

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
          Accounts, posts, and integration keys are being built.
          This is the application scaffold, not a working social network yet.
        </p>
      </section>
    </main>
  )
}
