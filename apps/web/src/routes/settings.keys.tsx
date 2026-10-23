import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery, useSuspenseQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { flushSync } from 'react-dom'
import { AccountForm, Field, passwordFrom } from '../components/account-form'
import { PlainText } from '../components/plain-text'
import { ReadFailure, SocialAction } from '../components/social'
import { ApiFailure, accountReady, type KeyList, type KeyMetadata } from '../lib/api'
import { sessionOptions } from '../lib/session'
import { codePoints } from '../lib/text'

export const Route = createFileRoute('/settings/keys')({ component: KeySettings })
const keysKey = ['keys'] as const
const permissions = ['profiles.read', 'posts.read', 'feed.read', 'profiles.write', 'posts.write', 'follows.write', 'likes.write'] as const
const readOnly = new Set<string>(['profiles.read', 'posts.read', 'feed.read'])

function KeySettings() {
  const { data: session } = useSuspenseQuery(sessionOptions)
  if (!session) return <main className="page"><h1>API keys</h1><Link to="/login">Log in to manage keys</Link></main>
  if (!accountReady(session)) return <main className="page"><h1>API keys</h1><Link to="/verify-email">Verify email before managing keys</Link></main>
  return <Keys key={session.id} />
}

function Keys() {
  const { api, queryClient } = Route.useRouteContext()
  const list = useQuery({ queryKey: keysKey, queryFn: ({ signal }) => api.read<KeyList>('/api/auth/keys', signal) })
  const [reauthenticated, setReauthenticated] = useState(false)
  // Deliberately not a mutation/query result: no key secret enters QueryCache,
  // MutationCache, route/search state, SSR hydration, or persistent storage.
  const [secret, setSecret] = useState<string>()
  const [copied, setCopied] = useState('')
  useEffect(() => {
    const clear = () => { flushSync(() => { setSecret(undefined); setCopied('') }) }
    window.addEventListener('pagehide', clear)
    return () => window.removeEventListener('pagehide', clear)
  }, [])
  if (list.isPending) return <main className="page"><h1>API keys</h1><p role="status">Loading keys…</p></main>
  if (!list.data) return <main className="page"><h1>API keys</h1>
    <ReadFailure error={list.error} retry={() => { void list.refetch() }} /></main>
  const available = list.data.integrationAvailable
  return <main className="page account-page">
    <h1>API keys</h1>
    <p>Personal keys automate social actions, not passwords or account management.</p>
    {!available && <p role="status">Key creation is unavailable: an operator must configure the Unkey integration. No fake keys are issued. Existing keys can still be revoked locally.</p>}
    {list.isError && <ReadFailure error={list.error} retry={() => { void list.refetch() }} />}
    {secret ? <section aria-labelledby="one-time-heading">
      <h2 id="one-time-heading">Save your key now</h2>
      <p>This secret is shown once. It cannot be recovered after dismissal or navigation. Keep it private.</p>
      <label htmlFor="one-time-secret">One-time API key</label>
      <input id="one-time-secret" value={secret} readOnly autoComplete="off" spellCheck={false} autoFocus />
      <div className="actions"><button type="button" onClick={async () => {
        try { await navigator.clipboard.writeText(secret); setCopied('Copied. Store the key privately.') }
        catch { setCopied('Clipboard unavailable. Select and copy the key manually before dismissing.') }
      }}>Copy key</button>
        <button type="button" onClick={() => { setSecret(undefined); setCopied('') }}>Dismiss secret</button></div>
      {copied && <p role="status">{copied}</p>}
    </section> : <>
      <h2>Confirm your password</h2>
      <p>Confirm before creating a key. The server permits recent password authentication for five minutes.</p>
      <AccountForm label="Confirm password" success="Password confirmed. You may create a key." disabled={!available}
        submit={async (form) => {
          await api.reauthenticate(passwordFrom(form))
          setReauthenticated(true)
        }}>
        <Field label="Current password" name="password" type="password" autoComplete="current-password" />
      </AccountForm>
      <h2>Create a key</h2>
      <p>Defaults are read-only with a 90-day expiry. Grant only what your integration needs.</p>
      {!reauthenticated && available && <p>Confirm your password above to enable creation.</p>}
      <AccountForm label="Create key" success="Key created. Save the one-time secret."
        disabled={!available || !reauthenticated} submit={async (form) => {
          const name = String(form.get('name') ?? '')
          const expiresInDays = Number(form.get('expiresInDays'))
          const selected = form.getAll('permissions').map(String)
          if (!name.trim() || codePoints(name) > 100 || /[\p{Cc}]/u.test(name) ||
            !Number.isInteger(expiresInDays) || expiresInDays < 1 || expiresInDays > 365 || selected.length === 0) {
            throw new ApiFailure(422, 'invalid_key', 'Choose a nonblank name up to 100 code points, at least one permission, and 1–365 days.')
          }
          try {
            const issued = await api.mutate<{ key: string; metadata: KeyMetadata }>('/api/auth/keys', 'POST', {
              name, permissions: selected, expiresInDays,
            })
            setSecret(issued.key)
            setReauthenticated(false)
            await queryClient.invalidateQueries({ queryKey: keysKey })
          } catch (error) {
            if (error instanceof ApiFailure && error.code === 'recent_password_required') setReauthenticated(false)
            throw error
          }
        }}>
        <Field label="Key name" name="name" autoComplete="off" />
        <div className="field"><label htmlFor="expiry-days">Expires in days</label>
          <input id="expiry-days" name="expiresInDays" type="number" min={1} max={365} defaultValue={90} required /></div>
        <fieldset className="permissions"><legend>Permissions</legend>
          {permissions.map((permission) => <label key={permission}>
            <input type="checkbox" name="permissions" value={permission} defaultChecked={readOnly.has(permission)} />
            {permission}
          </label>)}
        </fieldset>
      </AccountForm>
    </>}
    <h2>Your keys</h2>
    {list.data.items.length === 0 && <p>No keys yet.</p>}
    <ul className="key-list">{list.data.items.map((key) => <li key={key.keyId} className="post-card">
      <h3><PlainText text={key.name} /></h3>
      <p><PlainText text={key.maskedHint} /> · <PlainText text={key.keyId} /></p>
      <p>Permissions: {key.permissions.join(', ')}</p>
      <p>Expires <time dateTime={key.expiresAt}>{key.expiresAt.slice(0, 10)}</time></p>
      <p role="status">Status: {key.status.replaceAll('_', ' ')}</p>
      {!key.revokedAt && <SocialAction label={`Revoke ${key.name}`} action={async () => {
        await api.mutate(`/api/auth/keys/${key.keyId}`, 'DELETE')
        await queryClient.invalidateQueries({ queryKey: keysKey })
      }} />}
    </li>)}</ul>
    <p><a href="/docs/openapi.json">Download OpenAPI</a>. The gateway—not this browser port—serves key-authenticated /v1 requests.</p>
  </main>
}
