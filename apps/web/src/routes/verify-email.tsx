import { useState } from 'react'
import { useSuspenseQuery } from '@tanstack/react-query'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { AccountForm, tokenSearch, useAccount } from '../components/account-form'
import { sessionOptions } from '../lib/session'

export const Route = createFileRoute('/verify-email')({ validateSearch: tokenSearch, component: VerifyEmail })

function VerifyEmail() {
  const { token } = Route.useSearch()
  const [complete, setComplete] = useState(false)
  const { api, refreshSession } = useAccount()
  const { data: session } = useSuspenseQuery(sessionOptions)
  const navigate = useNavigate()
  return <main className="page account-page">
    <h1>Verify your email</h1>
    {complete ? <p role="status">Email verified. You can now log in and use protected features.</p> : token ?
      <AccountForm label="Verify email" success="Email verified."
        submit={() => api.verifyEmail(token)} afterSuccess={async () => {
          await refreshSession()
          await navigate({ to: '/verify-email', search: {}, replace: true })
          setComplete(true)
        }}>
        <p>Confirm to use this verification link. Opening the page alone does not consume it.</p>
      </AccountForm> :
      <p>Open the latest email link to verify your account. Links expire after 24 hours; a resend can replace an earlier link.</p>}
    {!complete && <section aria-labelledby="resend-heading">
      <h2 id="resend-heading">Need another verification email?</h2>
      {session ? <AccountForm label="Resend verification email" success="If verification is still needed, a new email has been queued."
        submit={() => api.resendVerification()} /> :
        <p><Link to="/login">Log in to request a fresh verification email.</Link></p>}
      {token && <p><Link to="/verify-email" search={{}}>Start again with a fresh link</Link></p>}
    </section>}
    <p><Link to="/login">Log in</Link> · <Link to="/">Return home</Link></p>
  </main>
}
