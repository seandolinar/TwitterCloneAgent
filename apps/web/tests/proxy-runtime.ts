import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createServer, request as httpRequest } from 'node:http'
import { test } from 'node:test'
import { setTimeout as delay } from 'node:timers/promises'

test('built Nitro ingress rejects raw path bypasses and overwrites public identity/client-IP headers', { timeout: 30_000 }, async (t) => {
  const received: Array<{ path: string; headers: Record<string, string | string[] | undefined> }> = []
  const backend = createServer((request, response) => {
    received.push({ path: request.url!, headers: request.headers })
    response.setHeader('Content-Type', 'application/json')
    response.setHeader('Set-Cookie', ['session=fixture; HttpOnly; Path=/', 'csrf=fixture; HttpOnly; Path=/'])
    response.end('{"csrfToken":"fixture"}')
  }).listen(0, '127.0.0.1')
  await once(backend, 'listening')
  t.after(() => backend.close())
  const backendAddress = backend.address()
  assert(backendAddress && typeof backendAddress !== 'string')
  const reservation = createServer().listen(0, '127.0.0.1')
  await once(reservation, 'listening')
  const address = reservation.address()
  assert(address && typeof address !== 'string')
  await new Promise<void>((resolve) => reservation.close(() => resolve()))
  const child = spawn(process.execPath, ['.output/server/index.mjs'], {
    cwd: new URL('..', import.meta.url),
    env: { PATH: process.env.PATH, HOST: '127.0.0.1', PORT: String(address.port),
      API_BASE_URL: `http://127.0.0.1:${backendAddress.port}`, NODE_ENV: 'production' },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let logs = ''
  child.stdout.on('data', (chunk) => { logs += chunk })
  child.stderr.on('data', (chunk) => { logs += chunk })
  t.after(async () => {
    if (child.exitCode !== null || child.signalCode !== null) return
    const exited = once(child, 'exit')
    child.kill('SIGTERM')
    const timer = setTimeout(() => child.kill('SIGKILL'), 3000)
    await exited
    clearTimeout(timer)
  })
  const web = `http://127.0.0.1:${address.port}`
  for (let attempt = 0; ; attempt++) {
    try {
      if ((await fetch(`${web}/healthz`)).ok) break
    } catch { /* process startup; bounded below */ }
    assert(attempt < 100 && child.exitCode === null, `built server did not start: ${logs}`)
    await delay(50)
  }
  function raw(path: string, method = 'GET', headers: Record<string, string> = {}, body = '') {
    return new Promise<{ status: number; cookies: string[]; body: string }>((resolve, reject) => {
      const request = httpRequest(web, { path, method, headers: { ...headers, 'Content-Length': String(Buffer.byteLength(body)) } }, (response) => {
        let text = ''
        response.setEncoding('utf8')
        response.on('data', (chunk) => { text += chunk })
        response.on('end', () => resolve({ status: response.statusCode!, cookies: response.headers['set-cookie'] ?? [], body: text }))
      })
      request.on('error', reject)
      request.end(body)
    })
  }
  const response = await raw('/api/auth/login', 'POST', {
    Origin: 'https://hostile.example', Cookie: 'session=fixture',
    'Content-Type': 'application/json', 'X-CSRF-Token': 'csrf-fixture',
    Authorization: 'Bearer forged-fixture', 'X-Unkey-Principal': 'forged-fixture',
    'X-Twitterclone-Client-IP': '203.0.113.1', 'X-Forwarded-For': '203.0.113.2',
    Forwarded: 'for=203.0.113.3', 'X-Forwarded-Host': 'evil.test',
  }, '{}')
  assert.equal(response.status, 200, response.body)
  assert.equal(response.cookies.length, 2)
  assert.equal(received.length, 1)
  assert.equal(received[0].path, '/browser/auth/login')
  assert.equal(received[0].headers.origin, 'https://hostile.example')
  assert.equal(received[0].headers['x-twitterclone-client-ip'], '127.0.0.1')
  for (const name of ['authorization', 'x-unkey-principal', 'x-forwarded-for', 'forwarded', 'x-forwarded-host']) {
    assert.equal(received[0].headers[name], undefined)
  }
  for (const path of [
    '/v1', '/v1/me', '/api/auth/%2e%2e/v1/posts', '/api/%2e/auth/login',
    '/api/%252e%252e/auth/login', '/api/auth/../auth/login', '/api//auth/login',
    '/api/auth%2flogin', '/api/auth\\login', '/api/http://evil.test',
    '/api/auth/session?url=http://evil.test', '/api/auth/login?redirect=https://evil.test',
  ]) {
    const result = await raw(path, 'POST', { 'Content-Type': 'application/json' }, '{}')
    assert([400, 404, 405, 422].includes(result.status), `${path}: ${result.status}`)
  }
  assert.equal(received.length, 1, 'path/method/query bypass must not contact the backend')
  const large = await raw('/api/auth/login', 'POST', { 'Content-Type': 'application/json' }, 'x'.repeat(16385))
  assert.equal(large.status, 413)
  assert.equal(received.length, 1)
})
