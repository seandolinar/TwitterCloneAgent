import { test, expect } from '@playwright/test'
import { fixturePassword } from './helpers'

test('form components support narrow screens, keyboard labels, announced validation, and password clearing', async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 700 })
  await page.goto('/register')
  await page.getByLabel('Email', { exact: true }).focus()
  await page.keyboard.type('keyboard-fixture@example.test')
  await page.keyboard.press('Tab')
  await expect(page.getByLabel('Handle', { exact: true })).toBeFocused()
  await page.keyboard.type('keyboard_fixture')
  await page.keyboard.press('Tab')
  await expect(page.getByLabel('Password', { exact: true })).toBeFocused()
  await page.keyboard.type('too short')
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Create account', exact: true })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('alert')).toHaveText('Passwords require 15–128 Unicode code points.')
  await expect(page.getByRole('alert')).toBeFocused()
  await expect(page.getByLabel('Password', { exact: true })).toHaveValue('')
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  const focus = await page.getByRole('alert').evaluate((element) => getComputedStyle(element).outlineStyle)
  expect(focus).not.toBe('none')
})

test('form components disable pending submission and do not persist passwords or session values', async ({ page, context }) => {
  const password = fixturePassword()
  let release!: () => void
  const gate = new Promise<void>((resolve) => { release = resolve })
  let entered!: () => void
  const received = new Promise<void>((resolve) => { entered = resolve })
  let submits = 0
  // Test-only transport fixture; this is not a runtime authorization bypass.
  await page.route('**/api/auth/register', async (route) => {
    submits++
    entered()
    await gate
    await route.fulfill({ status: 202, body: '' })
  })
  await page.goto('/register')
  await page.getByLabel('Email', { exact: true }).fill('pending-component@example.test')
  await page.getByLabel('Handle', { exact: true }).fill('pending_component')
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Create account', exact: true }).click()
  await received
  await expect(page.getByRole('button', { name: 'Please wait…', exact: true })).toBeDisabled()
  expect(submits).toBe(1)
  release()
  await expect(page.getByRole('status')).toContainText('If registration can be completed')
  await expect(page.getByLabel('Password', { exact: true })).toHaveValue('')
  const persisted = await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))
  const cookies = await context.cookies()
  expect(persisted.includes(password)).toBe(false)
  expect(cookies.filter((c) => /session|csrf/.test(c.name)).some((cookie) => persisted.includes(cookie.value))).toBe(false)
})

test('production headers allow hydration but block untrusted markup scripts and shared account caching', async ({ page }) => {
  const response = await page.goto('/login')
  expect(response).not.toBeNull()
  const headers = response!.headers()
  expect(headers['cache-control']).toContain('no-store')
  expect(headers['referrer-policy']).toBe('no-referrer')
  expect(headers['x-content-type-options']).toBe('nosniff')
  expect(headers['x-frame-options']).toBe('DENY')
  expect(headers['content-security-policy']).toContain("'nonce-")
  expect(headers['content-security-policy']).not.toContain("'unsafe-inline'")
  await expect(page.getByRole('button', { name: 'Log in', exact: true })).toBeEnabled()
  // strict-dynamic intentionally propagates trust to scripts created by trusted
  // JavaScript. Test untrusted parser-inserted markup, not privileged automation.
  await page.route('**/__csp-fixture', (route) => route.fulfill({
    status: 200,
    headers: { 'Content-Type': 'text/html', 'Content-Security-Policy': headers['content-security-policy'] },
    body: '<!doctype html><script>window.__markupInjectionFixture = true</script><p>Untrusted markup fixture</p>',
  }))
  await page.goto('/__csp-fixture')
  const executed = await page.evaluate(() => '__markupInjectionFixture' in window)
  expect(executed).toBe(false)
})
