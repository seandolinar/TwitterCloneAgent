import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { AccountForm, Field, useAccount } from '../components/account-form'

export const Route = createFileRoute('/login')({ component: Login })

function Login() {
  const { api } = useAccount()
  const navigate = useNavigate()
  return <main className="page account-page">
    <h1>Log in</h1>
    <AccountForm label="Log in" success="You are logged in." afterSuccess={async () => { await navigate({ to: '/' }) }}
      submit={(form) => api.login({ email: String(form.get('email')), password: String(form.get('password')) })}>
      <Field label="Email" name="email" type="email" autoComplete="email" />
      <Field label="Password" name="password" type="password" autoComplete="current-password" />
    </AccountForm>
    <p><Link to="/forgot-password">Forgot your password?</Link> · <Link to="/register">Create an account</Link></p>
  </main>
}
