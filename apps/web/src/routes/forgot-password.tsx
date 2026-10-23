import { createFileRoute, Link } from '@tanstack/react-router'
import { AccountForm, Field, useAccount } from '../components/account-form'

export const Route = createFileRoute('/forgot-password')({ component: ForgotPassword })

function ForgotPassword() {
  const { api } = useAccount()
  return <main className="page account-page">
    <h1>Reset your password</h1>
    <AccountForm label="Send reset link" success="If the address can receive account recovery email, a reset link has been queued."
      submit={(form) => api.requestReset(String(form.get('email')))}>
      <Field label="Email" name="email" type="email" autoComplete="email" />
    </AccountForm>
    <p>Reset links expire after 30 minutes. Delivery may be delayed; your current password works until a reset succeeds.</p>
    <p><Link to="/login">Return to login</Link></p>
  </main>
}
