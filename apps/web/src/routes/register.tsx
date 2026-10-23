import { createFileRoute, Link } from '@tanstack/react-router'
import { AccountForm, Field, passwordFrom, useAccount } from '../components/account-form'
import { useSuspenseQuery } from '@tanstack/react-query'
import { accountSettingsOptions } from '../lib/session'

export const Route = createFileRoute('/register')({ component: Register })

function Register() {
  const { api } = useAccount()
  const { data: settings } = useSuspenseQuery(accountSettingsOptions)
  const emailEnabled = settings.emailEnabled !== false
  return <main className="page account-page">
    <h1>Create an account</h1>
    <p>{emailEnabled ? 'Verify your email before posting, following, liking, or using a personalized feed.' : 'Create an account, then log in to start posting. Email verification is not required.'}</p>
    <AccountForm label="Create account" success={emailEnabled ? 'If registration can be completed, check your inbox for a verification email. You can then log in.' : 'If registration can be completed, your account is ready. Log in with your email and password.'}
      submit={(form) => api.register({ email: String(form.get('email')), handle: String(form.get('handle')), password: passwordFrom(form) })}>
      <Field label="Email" name="email" type="email" autoComplete="email" />
      <Field label="Handle" name="handle" autoComplete="username" pattern="[A-Za-z0-9_]{3,30}"
        hint="3–30 letters, digits, or underscores. Your handle cannot be changed." />
      <Field label="Password" name="password" type="password" autoComplete="new-password"
        hint="15–128 Unicode code points. Spaces are preserved." />
    </AccountForm>
    <p><Link to="/login">Log in</Link>{emailEnabled && <> · <Link to="/verify-email">Verification help</Link></>}</p>
  </main>
}
