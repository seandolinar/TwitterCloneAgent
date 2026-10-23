import { useState } from 'react'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { AccountForm, Field, passwordFrom, tokenSearch, useAccount } from '../components/account-form'
import { ApiFailure } from '../lib/api'
import { useSuspenseQuery } from '@tanstack/react-query'
import { accountSettingsOptions } from '../lib/session'

export const Route = createFileRoute('/reset-password')({ validateSearch: tokenSearch, component: ResetPassword })

function ResetPassword() {
  const { token } = Route.useSearch()
  const { api } = useAccount()
  const [complete, setComplete] = useState(false)
  const navigate = useNavigate()
  const { data: settings } = useSuspenseQuery(accountSettingsOptions)
  if (settings.emailEnabled === false) return <main className="page account-page">
    <h1>Password recovery unavailable</h1><p>Email password recovery is disabled for this app.</p>
    <p><Link to="/login">Return to login</Link></p>
  </main>
  return <main className="page account-page">
    <h1>Choose a new password</h1>
    {complete ? <p role="status">Password reset. Old sessions and API keys are revoked. Log in again with your new password.</p> : token ?
      <AccountForm label="Reset password" success="Password reset."
        submit={async (form) => {
          const password = passwordFrom(form)
          if (password !== String(form.get('confirmation'))) throw new ApiFailure(422, 'password_mismatch', 'Passwords do not match.')
          await api.resetPassword(token, password)
        }} afterSuccess={async () => {
          await navigate({ to: '/reset-password', search: {}, replace: true })
          setComplete(true)
        }}>
        <Field label="New password" name="password" type="password" autoComplete="new-password" hint="15–128 Unicode code points. Spaces are preserved." />
        <Field label="Confirm new password" name="confirmation" type="password" autoComplete="new-password" />
      </AccountForm> :
      <p>Open the latest reset link from your inbox. Links expire after 30 minutes and can only be used once.</p>}
    <p><Link to="/forgot-password">Request a fresh reset link</Link> · <Link to="/login">Log in</Link></p>
  </main>
}
