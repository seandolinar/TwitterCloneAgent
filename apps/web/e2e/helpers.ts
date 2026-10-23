import { expect, type Page, type APIRequestContext } from '@playwright/test'
import { randomUUID } from 'node:crypto'

export const fixturePassword = () => 'Fixture ' + randomUUID() + ' password'

export async function call(page: Page, method: string, path: string, body?: unknown, csrf?: string) {
  return page.evaluate(async ({ method, path, body, csrf }) => {
    const response = await fetch(path, {
      method, credentials: 'same-origin',
      headers: { ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
        ...(csrf === undefined ? {} : { 'X-CSRF-Token': csrf }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    const text = await response.text()
    return { status: response.status, data: text ? JSON.parse(text) : null }
  }, { method, path, body, csrf })
}

export async function mailToken(request: APIRequestContext, email: string, path: string): Promise<string> {
  const mail = process.env.MAIL_BASE_URL
  if (!mail) throw new Error('Use tests/with_compose.py to supply MAIL_BASE_URL.')
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    const response = await request.get(mail + '/api/v1/messages')
    const list = await response.json()
    for (const summary of list.messages) {
      if (!summary.To?.some((target: { Address: string }) => target.Address === email)) continue
      const message = await (await request.get(mail + '/api/v1/message/' + summary.ID)).json()
      for (const line of message.Text.split('\n')) {
        const value = line.trim()
        if (value.startsWith(process.env.WEB_BASE_URL + path + '?')) {
          const token = new URL(value).searchParams.get('token')
          if (token) return token
        }
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error('Expected fixture email was not delivered before its deadline.')
}

export async function onboard(page: Page, request: APIRequestContext) {
  const suffix = randomUUID().replaceAll('-', '').slice(0, 16)
  const handle = 'fixture_' + suffix
  const email = handle + '@example.test'
  const password = fixturePassword()
  await page.goto('/')
  let result = await call(page, 'GET', '/api/auth/csrf')
  const csrf = result.data.csrfToken as string
  result = await call(page, 'POST', '/api/auth/register', { email, handle, password }, csrf)
  expect(result.status).toBe(202)
  const token = await mailToken(request, email, '/verify-email')
  result = await call(page, 'POST', '/api/auth/verify-email', { token }, csrf)
  expect(result.status).toBe(204)
  result = await call(page, 'POST', '/api/auth/login', { email, password }, csrf)
  expect(result.status).toBe(200)
  return { email, handle, password, csrf: result.data.csrfToken as string }
}
