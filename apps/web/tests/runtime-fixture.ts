import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createServer, request as httpRequest } from 'node:http'
import { setTimeout as delay } from 'node:timers/promises'
import type { TestContext } from 'node:test'

export async function builtServer(t: TestContext, env: Record<string, string>) {
  const reservation = createServer().listen(0, '127.0.0.1')
  await once(reservation, 'listening')
  const address = reservation.address()
  assert(address && typeof address !== 'string')
  await new Promise<void>((resolve) => reservation.close(() => resolve()))
  const child = spawn(process.execPath, ['.output/server/index.mjs'], {
    cwd: new URL('..', import.meta.url), stdio: ['ignore', 'pipe', 'pipe'],
    env: { PATH: process.env.PATH, HOST: '127.0.0.1', PORT: String(address.port), NODE_ENV: 'production', ...env },
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
  const origin = `http://127.0.0.1:${address.port}`
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(origin + '/healthz')).ok) break } catch { /* startup */ }
    assert(attempt < 100 && child.exitCode === null, `Built server startup failed: ${logs}`)
    await delay(50)
  }
  return { origin, raw(path: string, method = 'GET', headers: Record<string, string> = {}, body = '') {
    return new Promise<{ status: number; body: string }>((resolve, reject) => {
      const request = httpRequest(origin, { path, method, headers }, (response) => {
        let text = ''
        response.on('data', (chunk) => { text += chunk })
        response.on('end', () => resolve({ status: response.statusCode!, body: text }))
      })
      request.on('error', reject)
      request.end(body)
    })
  } }
}
