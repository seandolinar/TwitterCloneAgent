import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { AccountForm, Field, useAccount } from '../components/account-form'
import { useSuspenseQuery } from '@tanstack/react-query'
import { accountSettingsOptions } from '../lib/session'

export const Route = createFileRoute('/login')({ component: Login })

function Login() {
  const { api } = useAccount()
  const navigate = useNavigate()
  const { data: settings } = useSuspenseQuery(accountSettingsOptions)
  return <main className="page account-page">
    <h1>Log in</h1>
    <AccountForm label="Log in" success="You are logged in." afterSuccess={async () => { await navigate({ to: '/' }) }}
      submit={(form) => api.login({ email: String(form.get('email')), password: String(form.get('password')) })}>
      <Field label="Email" name="email" type="email" autoComplete="email" />
      <Field label="Password" name="password" type="password" autoComplete="current-password" />
    </AccountForm>
    <p>{settings.emailEnabled !== false && <><Link to="/forgot-password">Forgot your password?</Link> · </>}<Link to="/register">Create an account</Link></p>
    {settings.emailEnabled === false && <p>Email password recovery is unavailable. Keep your password safe.</p>}
  </main>
}
