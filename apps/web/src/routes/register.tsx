import { createFileRoute, Link } from '@tanstack/react-router'
import { AccountForm, Field, passwordFrom, useAccount } from '../components/account-form'

export const Route = createFileRoute('/register')({ component: Register })

function Register() {
  const { api } = useAccount()
  return <main className="page account-page">
    <h1>Create an account</h1>
    <p>Verify your email before posting, following, liking, or using a personalized feed.</p>
    <AccountForm label="Create account" success="If registration can be completed, check your inbox for a verification email. You can then log in."
      submit={(form) => api.register({ email: String(form.get('email')), handle: String(form.get('handle')), password: passwordFrom(form) })}>
      <Field label="Email" name="email" type="email" autoComplete="email" />
      <Field label="Handle" name="handle" autoComplete="username" pattern="[A-Za-z0-9_]{3,30}"
        hint="3–30 letters, digits, or underscores. Your handle cannot be changed." />
      <Field label="Password" name="password" type="password" autoComplete="new-password"
        hint="15–128 Unicode code points. Spaces are preserved." />
    </AccountForm>
    <p><Link to="/login">Log in</Link> · <Link to="/verify-email">Verification help</Link></p>
  </main>
}
