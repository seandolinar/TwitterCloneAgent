import { test, expect } from '@playwright/test'
import { createServer } from 'node:http'
import { once } from 'node:events'
import { call, onboard } from './helpers'
import { readSocialScenario, writeSocialScenario } from './social-scenarios'
import { keySettingsScenario } from './key-scenarios'

test.beforeAll(() => {
  if (!process.env.WEB_BASE_URL || !process.env.MAIL_BASE_URL) {
    throw new Error('Run this suite with python3 tests/with_compose.py npm --prefix apps/web run test:e2e')
  }
})

test('two-account acceptance covers SSR isolation, social actions, and one-time key settings', async ({ browser, request }) => {
  test.setTimeout(120_000)
  const first = await browser.newContext()
  const second = await browser.newContext()
  try {
    const alicePage = await first.newPage()
    const bobPage = await second.newPage()
    const alice = await onboard(alicePage, request)
    const bob = await onboard(bobPage, request)
    const [firstResponse, secondResponse] = await Promise.all([
      first.request.get(process.env.WEB_BASE_URL!),
      second.request.get(process.env.WEB_BASE_URL!),
    ])
    expect(firstResponse.status()).toBe(200)
    expect(secondResponse.status()).toBe(200)
    expect(firstResponse.headers()['cache-control']).toContain('no-store')
    const [firstHtml, secondHtml] = await Promise.all([firstResponse.text(), secondResponse.text()])
    expect(firstHtml).toContain(alice.handle)
    expect(firstHtml).not.toContain(bob.handle)
    expect(secondHtml).toContain(bob.handle)
    expect(secondHtml).not.toContain(alice.handle)
    await alicePage.reload()
    await expect(alicePage.getByText('Signed in as @' + alice.handle, { exact: true })).toBeVisible()
    const cookies = await first.cookies()
    expect(cookies.filter((cookie) => /session|csrf/.test(cookie.name)).every((cookie) => cookie.httpOnly)).toBe(true)
    expect(await alicePage.evaluate(() => document.cookie)).not.toContain('twitterclone_dev_session')
    expect(await alicePage.evaluate(() => document.cookie)).not.toContain('twitterclone_dev_csrf')
    // Reuse these real accounts so the complete suite still respects the
    // default registration/IP budget rather than weakening production limits.
    await readSocialScenario(browser, alicePage, bobPage, alice, bob)
    await writeSocialScenario(alicePage, bobPage, alice, bob)
    await keySettingsScenario(alicePage, alice)
  } finally {
    await first.close()
    await second.close()
  }
})

test('browser security rejects hostile-origin and missing-CSRF mutations without saving a post', async ({ page, context, request, browser }) => {
  const account = await onboard(page, request)
  const baseline = await call(page, 'GET', '/api/v1/me')
  const missing = await call(page, 'POST', '/api/v1/posts', { body: 'must not save' })
  expect(missing.status).toBe(403)
  // A valid live session and token cannot compensate for the wrong Origin.
  const hostile = await context.request.post(process.env.WEB_BASE_URL + '/api/v1/posts', {
    headers: { Origin: 'http://localhost:1', 'X-CSRF-Token': account.csrf },
    data: { body: 'must not save either' },
  })
  expect(hostile.status()).toBe(403)
  // Also exercise an actual same-site, cross-origin browser POST with cookies.
  const attacker = createServer((_req, res) => res.end('<!doctype html><title>Hostile fixture</title>')).listen(0, '127.0.0.1')
  await once(attacker, 'listening')
  try {
    const address = attacker.address()
    if (!address || typeof address === 'string') throw new Error('Fixture address unavailable')
    const attackerPage = await context.newPage()
    await attackerPage.goto(`http://localhost:${address.port}`)
    const denial = attackerPage.waitForResponse((response) => response.url() === process.env.WEB_BASE_URL + '/api/v1/posts')
    await attackerPage.evaluate(async (origin) => {
      await fetch(origin + '/api/v1/posts', { method: 'POST', mode: 'no-cors', credentials: 'include',
        headers: { 'Content-Type': 'text/plain' }, body: '{"body":"cross-origin fixture"}' })
    }, process.env.WEB_BASE_URL!)
    expect((await denial).status()).toBe(403)
    await attackerPage.close()
    const after = await call(page, 'GET', '/api/v1/me')
    expect(after.data.postCount).toBe(baseline.data.postCount)
  } finally { attacker.close() }
  const clean = await browser.newContext()
  try {
    const cleanPage = await clean.newPage()
    await cleanPage.goto('/')
    await expect(cleanPage.getByText('Signed out', { exact: true })).toBeVisible()
    expect((await call(cleanPage, 'GET', '/api/auth/session')).status).toBe(401)
  } finally { await clean.close() }
})
