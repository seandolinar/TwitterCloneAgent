import { createFileRoute, Link } from '@tanstack/react-router'
import { AccountForm, useAccount } from '../components/account-form'

export const Route = createFileRoute('/logout')({ component: Logout })

function Logout() {
  const { api } = useAccount()
  return <main className="page account-page">
    <h1>Log out</h1>
    <p>Confirm to revoke this browser session. Opening this page alone does not log you out.</p>
    <AccountForm label="Confirm logout" success="You are logged out." submit={() => api.logout()} />
    <p><Link to="/login">Log in</Link> · <Link to="/">Return home</Link></p>
  </main>
}
