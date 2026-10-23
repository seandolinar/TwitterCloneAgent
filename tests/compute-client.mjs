// Network-isolated managed-ingress fixture, NOT a real Unkey verifier.
import assert from 'node:assert/strict'
import { request as rawRequest } from 'node:http'
const prod = process.env.PROD_URL
const preview = process.env.PREVIEW_URL
const principal = process.env.PROD_PRINCIPAL
const previewPrincipal = process.env.PREVIEW_PRINCIPAL
const context = { 'X-Forwarded-For': '203.0.113.7', 'X-Forwarded-Proto': 'https' }
async function call(origin, path, { method = 'GET', headers = {}, body } = {}) {
  return fetch(origin + path, { method, redirect: 'manual',
    signal: AbortSignal.timeout(5000), headers: { ...context, ...headers,
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
    body: body === undefined ? undefined : JSON.stringify(body) })
}
const key = { 'X-Unkey-Principal': principal }
if (process.env.PHASE === 'revoked') {
  assert.equal((await call(prod, '/v1/me', { headers: key })).status, 401)
  assert.equal((await call(preview, '/v1/me', { headers: { 'X-Unkey-Principal': previewPrincipal } })).status, 200)
} else {
  assert.equal((await call(prod, '/readyz')).status, 200)
  assert.equal((await call(preview, '/readyz')).status, 200)
  assert.equal((await call(prod, '/docs/openapi.json')).status, 200)
  assert.equal((await call(prod, '/v1/me')).status, 401)
  assert.equal((await call(prod, '/v1/me', { headers: { Cookie: process.env.SESSION_COOKIE } })).status, 401)
  const me = await call(prod, '/v1/me', { headers: key })
  assert.equal(me.status, 200)
  assert.equal((await me.json()).id, process.env.PROD_USER)
  assert.equal((await call(preview, '/v1/me', { headers: key })).status, 401)
  const previewFeed = await call(preview, '/v1/feed', { headers: { 'X-Unkey-Principal': previewPrincipal } })
  assert.equal(previewFeed.status, 200)
  assert.equal((await previewFeed.json()).items.length, 0)
  if (process.env.PHASE === 'initial') {
    assert.equal((await call(prod, '/v1/posts', { method: 'POST', headers: key,
      body: { body: 'Combined integration fixture post' } })).status, 201)
    assert.equal((await call(prod, '/v1/me/following/' + process.env.TARGET_USER,
      { method: 'PUT', headers: key })).status, 204)
    assert.equal((await call(prod, '/v1/posts/' + process.env.TARGET_POST + '/like',
      { method: 'PUT', headers: key })).status, 204)
    // A principal on the browser branch is stripped; the cookie alone owns identity.
    const browserHeaders = { Cookie: process.env.SESSION_COOKIE, 'X-Unkey-Principal': previewPrincipal,
      Origin: 'https://production.example.com', 'X-Twitterclone-Client-IP': '198.51.100.99' }
    const browserMe = await call(prod, '/api/v1/me', { headers: browserHeaders })
    assert.equal(browserMe.status, 200)
    assert.equal((await browserMe.json()).id, process.env.PROD_USER)
    assert.equal((await call(prod, '/api/v1/me', { headers: key })).status, 401)
    assert.equal((await call(prod, '/api/v1/posts', { method: 'POST', headers: browserHeaders,
      body: { body: 'Missing CSRF must not save' } })).status, 403)
    const bootstrap = await call(prod, '/api/auth/csrf', { headers: browserHeaders })
    assert.equal(bootstrap.status, 200)
    const csrf = (await bootstrap.json()).csrfToken
    const cookies = [process.env.SESSION_COOKIE, ...bootstrap.headers.getSetCookie().map((cookie) => cookie.split(';')[0])].join('; ')
    assert.equal((await call(prod, '/api/v1/posts', { method: 'POST', headers: { ...browserHeaders,
      Cookie: cookies, 'X-CSRF-Token': csrf }, body: { body: 'Combined browser fixture post' } })).status, 201)
    for (const port of [8081, 8082]) {
      let exposed = false
      try { await fetch(`http://combined-production:${port}/healthz`, { signal: AbortSignal.timeout(1000) }); exposed = true } catch {}
      assert.equal(exposed, false, 'Rust loopback listener must not be reachable from the fixture network')
    }
    for (const path of ['/v1/%2e%2e/api/auth/session', '/api/auth/%252e%252e/session', '/v1//me']) {
      const status = await new Promise((resolve, reject) => {
        const request = rawRequest(prod, { path, headers: { ...context, ...key } }, (response) => {
          response.resume(); resolve(response.statusCode)
        })
        request.on('error', reject); request.end()
      })
      assert.equal(status, 400)
    }
  }
  const feed = await call(prod, '/v1/feed', { headers: key })
  assert.equal(feed.status, 200)
  assert.equal((await feed.json()).items.length, 3)
  const owner = await (await call(prod, '/v1/me', { headers: key })).json()
  assert.equal(owner.postCount, 2)
  assert.equal(owner.followingCount, 1)
  const liked = await (await call(prod, '/v1/posts/' + process.env.TARGET_POST, { headers: key })).json()
  assert.equal(liked.likeCount, 1)
}
console.log('PASS: combined image ' + process.env.PHASE + ' fixture checks (not live Unkey/Compute)')
