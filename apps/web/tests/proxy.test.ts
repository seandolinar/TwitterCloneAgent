import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { once } from 'node:events'
import { test } from 'node:test'
import { backendOrigin, canonicalTarget, createServerApi, proxyBrowserRequest } from '../src/lib/browser-proxy.server.ts'

test('backend origin and canonical paths reject ambiguous destinations and encoding', () => {
  assert.equal(backendOrigin('http://api:8080').href, 'http://api:8080/')
  for (const origin of ['https://user:private@example.test', 'http://api/path', 'file:///etc/passwd', 'http://api?url=private']) {
    assert.throws(() => backendOrigin(origin))
  }
  for (const path of ['/api/%2e%2e/auth/login', '/api/auth/../v1/posts', '/api//auth/login', '//evil.test', '/api/%252e%252e', '/api\\auth\\login']) {
    assert.equal(canonicalTarget(path), false)
  }
  assert.equal(canonicalTarget('/api/v1/feed?cursor=abc%2Fdef'), true)
})

test('proxy allowlist strips identity headers, preserves actual Origin, and relays cookies separately', async (t) => {
  const received: Array<{ path: string; headers: Record<string, string | string[] | undefined> }> = []
  const server = createServer((request, response) => {
    received.push({ path: request.url!, headers: request.headers })
    response.setHeader('Set-Cookie', ['session=fixture; HttpOnly; Path=/', 'csrf=fixture; HttpOnly; Path=/'])
    response.setHeader('Content-Type', 'application/json')
    response.end('{"ok":true}')
  }).listen(0, '127.0.0.1')
  await once(server, 'listening')
  t.after(() => server.close())
  const address = server.address()
  assert(address && typeof address !== 'string')
  const backend = backendOrigin(`http://127.0.0.1:${address.port}`)
  const response = await proxyBrowserRequest(new Request('http://web/api/auth/login', {
    method: 'POST', body: '{}',
    headers: {
      Cookie: 'session=fixture', Origin: 'https://hostile.example',
      'X-CSRF-Token': 'csrf-fixture', Authorization: 'Bearer forged',
      'X-Unkey-Principal': 'forged', 'X-Twitterclone-Client-IP': '203.0.113.1',
      'X-Forwarded-For': '203.0.113.2', Forwarded: 'for=203.0.113.3',
    },
  }), '192.0.2.1', backend)
  assert.equal(response.status, 200)
  assert.equal(response.headers.getSetCookie().length, 2)
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.equal(received[0].path, '/browser/auth/login')
  assert.equal(received[0].headers.origin, 'https://hostile.example')
  assert.equal(received[0].headers.cookie, 'session=fixture')
  assert.equal(received[0].headers['x-csrf-token'], 'csrf-fixture')
  assert.equal(received[0].headers['x-twitterclone-client-ip'], '192.0.2.1')
  for (const name of ['authorization', 'x-unkey-principal', 'x-forwarded-for', 'forwarded']) {
    assert.equal(received[0].headers[name], undefined)
  }
  for (const path of ['/v1/me', '/api/http://evil.test', '/api/auth/credentials', '/api/v1/feed?url=http://evil.test']) {
    assert.notEqual((await proxyBrowserRequest(new Request(`http://web${path}`), '192.0.2.1', backend)).status, 200)
  }
  assert.equal((await proxyBrowserRequest(new Request('http://web/api/auth/login'), '192.0.2.1', backend)).status, 405)
  assert.equal(received.length, 1)
})

test('body and dependency errors are bounded, redacted, and never followed to another host', async (t) => {
  let requests = 0
  const server = createServer((_request, response) => {
    requests++
    response.writeHead(302, { Location: 'https://private-redirect.example' }).end()
  }).listen(0, '127.0.0.1')
  await once(server, 'listening')
  t.after(() => server.close())
  const address = server.address()
  assert(address && typeof address !== 'string')
  const backend = backendOrigin(`http://127.0.0.1:${address.port}`)
  const large = await proxyBrowserRequest(new Request('http://web/api/auth/login', { method: 'POST', body: 'x'.repeat(16385) }), '192.0.2.1', backend)
  assert.equal(large.status, 413)
  assert.equal(requests, 0)
  const redirect = await proxyBrowserRequest(new Request('http://web/api/auth/login', { method: 'POST', body: '{}' }), '192.0.2.1', backend)
  assert.equal(redirect.status, 503)
  assert(!JSON.stringify(await redirect.json()).includes('private-redirect'))
  assert.equal(requests, 1)
  const failed = await proxyBrowserRequest(new Request('http://web/api/auth/session?private=secret'), '192.0.2.1', backendOrigin('http://127.0.0.1:1'))
  assert(!JSON.stringify(await failed.json()).includes('secret'))
})

test('SSR read closures isolate cookies and cannot become arbitrary fetchers or mutate state', async (t) => {
  const server = createServer((request, response) => {
    response.setHeader('Content-Type', 'application/json')
    setTimeout(() => response.end(JSON.stringify({ cookie: request.headers.cookie, method: request.method })), 10)
  }).listen(0, '127.0.0.1')
  await once(server, 'listening')
  t.after(() => server.close())
  const address = server.address()
  assert(address && typeof address !== 'string')
  const previous = process.env.API_BASE_URL
  process.env.API_BASE_URL = `http://127.0.0.1:${address.port}`
  t.after(() => {
    if (previous === undefined) delete process.env.API_BASE_URL
    else process.env.API_BASE_URL = previous
  })
  function request(cookie: string) {
    const request = new Request('http://web/profile/Alice', { headers: { Cookie: cookie } })
    Object.defineProperty(request, 'runtime', { value: { name: 'node', node: {
      req: { socket: { remoteAddress: '192.0.2.1' }, url: '/profile/Alice' },
    } } })
    return request
  }
  const alice = createServerApi(request('session=alice-fixture'))
  const bob = createServerApi(request('session=bob-fixture'))
  const [first, second] = await Promise.all([alice.read('/api/auth/session'), bob.read('/api/auth/session')])
  assert.deepEqual(await first.json(), { cookie: 'session=alice-fixture', method: 'GET' })
  assert.deepEqual(await second.json(), { cookie: 'session=bob-fixture', method: 'GET' })
  assert.equal((await alice.read('https://evil.test/api/auth/session')).status, 400)
  assert.equal((await alice.read('/api/v1/posts')).status, 405)
  assert.equal((await createServerApi(new Request('http://web/')).read('/api/auth/session')).status, 503)
})
