import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { once } from 'node:events'
import { test } from 'node:test'
import { readFileSync } from 'node:fs'
import { builtServer } from './runtime-fixture.ts'

test('built Compute ingress separates authentication, rejects bypasses, and checks Rust readiness', { timeout: 30_000 }, async (t) => {
  const received: Array<{ listener: string; path: string; headers: Record<string, string | string[] | undefined> }> = []
  let ready = false
  async function backend(listener: string) {
    const server = createServer((request, response) => {
      received.push({ listener, path: request.url!, headers: request.headers })
      if (request.url === '/readyz') { response.writeHead(ready ? 200 : 503).end(); return }
      if (listener === 'browser' && !request.headers.cookie) { response.writeHead(401).end('{}'); return }
      response.setHeader('Content-Type', 'application/json')
      response.end('{}')
    }).listen(0, '127.0.0.1')
    await once(server, 'listening')
    t.after(() => server.close())
    const address = server.address()
    assert(address && typeof address !== 'string')
    return `http://127.0.0.1:${address.port}`
  }
  const api = await backend('browser')
  const gateway = await backend('gateway')
  const web = await builtServer(t, { WEB_RUNTIME_MODE: 'compute', API_BASE_URL: api, GATEWAY_API_BASE_URL: gateway })
  const context = { 'X-Forwarded-For': '203.0.113.9', 'X-Forwarded-Proto': 'https' }
  const principal = '{"version":"v1","fixture":"gateway-generated"}'
  let response = await web.raw('/api/v1/me', 'GET', { ...context, Cookie: 'session=fixture',
    Authorization: 'Bearer must-not-forward', 'X-Unkey-Principal': principal,
    'X-Twitterclone-Client-IP': '198.51.100.10', Origin: 'https://actual-origin.example' })
  assert.equal(response.status, 200)
  assert.equal(received[0].listener, 'browser')
  assert.equal(received[0].path, '/browser/v1/me')
  assert.equal(received[0].headers.origin, 'https://actual-origin.example')
  assert.equal(received[0].headers['x-twitterclone-client-ip'], '203.0.113.9')
  assert.equal(received[0].headers['x-unkey-principal'], undefined)
  assert.equal(received[0].headers.authorization, undefined)
  response = await web.raw('/api/v1/me', 'GET', { ...context, 'X-Unkey-Principal': principal })
  assert.equal(response.status, 401, 'principal cannot authorize browser requests')
  response = await web.raw('/v1/me', 'GET', { ...context, Cookie: 'session=fixture' })
  assert.equal(response.status, 401, 'cookies cannot authorize integration requests')
  const baseline = received.length
  response = await web.raw('/v1/me', 'GET', { ...context, 'X-Unkey-Principal': principal,
    Cookie: 'session=must-not-forward', Authorization: 'Bearer must-not-forward', 'X-CSRF-Token': 'must-not-forward' })
  assert.equal(response.status, 200)
  assert.equal(received.length, baseline + 1)
  const record = received[baseline]
  assert.equal(record.listener, 'gateway')
  assert.equal(record.headers['x-unkey-principal'], principal)
  for (const name of ['cookie', 'authorization', 'x-csrf-token', 'x-forwarded-for']) assert.equal(record.headers[name], undefined)
  const contract = JSON.parse(readFileSync(new URL('../public/docs/openapi.json', import.meta.url), 'utf8'))
  let operations = 0
  for (const [path, item] of Object.entries(contract.paths)) {
    for (const method of Object.keys(item as object)) {
      const concrete = path.replace('{handle}', 'alice').replace('{id}', '00000000-0000-0000-0000-000000000001')
      response = await web.raw(concrete, method.toUpperCase(), { ...context, 'X-Unkey-Principal': principal,
        'Content-Type': 'application/json' }, method === 'post' || method === 'patch' ? '{}' : '')
      assert.equal(response.status, 200, `${method} ${path} must forward`)
      assert.equal(received[received.length - 1].listener, 'gateway')
      operations++
    }
  }
  assert.equal(operations, 12)
  const beforeBypasses = received.length
  for (const path of ['/v1', '/v1/auth/keys', '/v1/%2eme', '/v1/../api/auth/keys',
    '/v1//me', '/v1\\me', '/api/%2e%2e/v1/me', '/v1/me?url=https://evil.example',
    '/v1/feed?cursor=one&cursor=two']) {
    response = await web.raw(path, 'GET', { ...context, 'X-Unkey-Principal': principal })
    assert([400, 404, 422].includes(response.status), `${path}: ${response.status}`)
  }
  assert.equal(received.length, beforeBypasses)
  const invalidContexts: Array<Record<string, string>> = [
    { 'X-Forwarded-For': '203.0.113.9, 198.51.100.1', 'X-Forwarded-Proto': 'https' },
    { 'X-Forwarded-For': '203.0.113.9', 'X-Forwarded-Proto': 'http' },
    {},
  ]
  for (const headers of invalidContexts) {
    assert.equal((await web.raw('/v1/me', 'GET', { ...headers, 'X-Unkey-Principal': principal })).status, 503)
  }
  assert.equal(received.length, beforeBypasses, 'invalid context must not reach Rust')
  assert.equal((await web.raw('/readyz')).status, 503)
  ready = true
  assert.equal((await web.raw('/readyz')).status, 200)
  ready = false
  assert.equal((await web.raw('/readyz')).status, 503)
})
