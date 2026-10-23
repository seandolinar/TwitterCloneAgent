import assert from 'node:assert/strict'
import { test } from 'node:test'
import { ApiFailure, createApiClient } from '../src/lib/api.ts'
import { createQueryState } from '../src/lib/query-state.ts'
import { QueryObserver } from '@tanstack/react-query'

test('CSRF bootstrap is deduplicated and login/logout refresh the in-memory context', async () => {
  const requests: Array<{ path: string; token: string | null }> = []
  let identityChanges = 0
  const api = createApiClient(async (path, init) => {
    requests.push({ path, token: new Headers(init?.headers).get('X-CSRF-Token') })
    assert.equal(init?.credentials, 'same-origin')
    assert.equal(init?.cache, 'no-store')
    if (path.endsWith('/csrf')) return Response.json({ csrfToken: 'anonymous-fixture' })
    if (path.endsWith('/login')) return Response.json({ csrfToken: 'session-fixture' })
    if (path.endsWith('/logout')) return Response.json({ csrfToken: 'new-anonymous-fixture' })
    return new Response(null, { status: 202 })
  }, async () => { identityChanges++ })
  await Promise.all([api.refreshCsrf(), api.refreshCsrf()])
  await api.login({ email: 'fixture@example.test', password: 'long fixture password' })
  await api.logout()
  await api.requestReset('fixture@example.test')
  assert.equal(requests.filter((r) => r.path.endsWith('/csrf')).length, 1)
  assert.equal(requests.find((r) => r.path.endsWith('/login'))?.token, 'anonymous-fixture')
  assert.equal(requests.find((r) => r.path.endsWith('/logout'))?.token, 'session-fixture')
  assert.equal(requests.find((r) => r.path.endsWith('/forgot-password'))?.token, 'new-anonymous-fixture')
  assert.equal(identityChanges, 2)
})

test('only explicit CSRF rejection retries a mutation; transport and dependency errors never retry writes', async () => {
  let csrfReads = 0
  let writes = 0
  const api = createApiClient(async (path) => {
    if (path.endsWith('/csrf')) return Response.json({ csrfToken: `csrf-${++csrfReads}` })
    if (++writes === 1) return Response.json({ error: { code: 'csrf_rejected' } }, { status: 403 })
    return new Response(null, { status: 204 })
  })
  await api.verifyEmail('fixture')
  assert.equal(writes, 2)
  assert.equal(csrfReads, 2)
  let attempts = 0
  const failed = createApiClient(async (path) => {
    if (path.endsWith('/csrf')) return Response.json({ csrfToken: 'fixture' })
    attempts++
    throw new Error('ambiguous network failure fixture')
  })
  await assert.rejects(failed.verifyEmail('fixture'))
  assert.equal(attempts, 1)
  for (const path of ['https://evil.test/api/auth/session', '/api/../v1/me', '//evil.test/api/auth/session']) {
    await assert.rejects(api.read(path as `/api/${string}`), ApiFailure)
  }
})

test('query/session state is request-scoped and identity change cancels and removes old account data', async () => {
  const first = createQueryState()
  const second = createQueryState()
  first.queryClient.setQueryData(['session'], { id: 'alice' })
  first.queryClient.setQueryData(['feed', 'alice'], ['private-alice-fixture'])
  second.queryClient.setQueryData(['session'], { id: 'bob' })
  assert.notEqual(first.queryClient, second.queryClient)
  assert.equal(second.queryClient.getQueryData(['feed', 'alice']), undefined)
  const originalFetch = globalThis.fetch
  globalThis.fetch = async (input) => String(input).endsWith('/csrf')
    ? Response.json({ csrfToken: 'fixture' }) : Response.json({ csrfToken: 'rotated-fixture' })
  try {
    await first.api.logout()
    assert.equal(first.queryClient.getQueryData(['session']), null)
    assert.equal(first.queryClient.getQueryData(['feed', 'alice']), undefined)
    assert.deepEqual(second.queryClient.getQueryData(['session']), { id: 'bob' })
  } finally {
    globalThis.fetch = originalFetch
    first.queryClient.clear()
    second.queryClient.clear()
  }
})

test('mounted session observers receive the new account instead of retaining a removed query', async () => {
  const state = createQueryState()
  state.queryClient.setQueryData(['session'], { id: 'alice' })
  state.queryClient.setQueryData(['feed', 'alice'], ['old private fixture'])
  const observer = new QueryObserver(state.queryClient, {
    queryKey: ['session'], queryFn: async () => ({ id: 'bob' }),
  })
  const identities: Array<string | null> = []
  const unsubscribe = observer.subscribe((result) => identities.push(result.data?.id ?? null))
  const originalFetch = globalThis.fetch
  globalThis.fetch = async () => Response.json({ csrfToken: 'fixture' })
  try {
    await state.api.login({ email: 'bob@example.test', password: 'long fixture password' })
    assert(identities.includes(null))
    assert.equal(observer.getCurrentResult().data?.id, 'bob')
    assert.equal(state.queryClient.getQueryData(['feed', 'alice']), undefined)
  } finally {
    globalThis.fetch = originalFetch
    unsubscribe()
    state.queryClient.clear()
  }
})
