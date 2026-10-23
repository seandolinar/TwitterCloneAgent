import { test, expect, type Page } from '@playwright/test'
import { randomUUID } from 'node:crypto'
import { call, fixturePassword, mailToken } from './helpers'

async function login(page: Page, email: string, password: string) {
  await page.goto('/login')
  await page.getByLabel('Email', { exact: true }).fill(email)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Log in', exact: true }).click()
}

async function openTokenPage(page: Page, path: string, token: string) {
  // Do not surface a token-bearing URL in a navigation failure's diagnostics.
  try { await page.goto(path + '?token=' + encodeURIComponent(token)) } catch {
    throw new Error('The fixture email link page did not load.')
  }
}

test.beforeAll(() => {
  if (!process.env.WEB_BASE_URL || !process.env.MAIL_BASE_URL) {
    throw new Error('Use tests/with_compose.py to run against an isolated real stack.')
  }
})

test('account screens onboard, verify, recover, revoke old sessions, and log out', async ({ page, browser, request }) => {
  test.setTimeout(60_000)
  const suffix = randomUUID().replaceAll('-', '').slice(0, 16)
  const handle = 'ui_' + suffix
  const email = handle + '@example.test'
  const password = fixturePassword()
  const replacement = fixturePassword()
  await page.goto('/register')
  await page.getByLabel('Email', { exact: true }).fill(email)
  await page.getByLabel('Handle', { exact: true }).fill(handle)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Create account', exact: true }).click()
  await expect(page.getByRole('status')).toHaveText('If registration can be completed, check your inbox for a verification email. You can then log in.')
  const verification = await mailToken(request, email, '/verify-email')
  await openTokenPage(page, '/verify-email', verification)
  // Opening a link is not consumption; explicit confirmation is required.
  await expect(page.getByRole('button', { name: 'Verify email', exact: true })).toBeEnabled()
  await page.getByRole('button', { name: 'Verify email', exact: true }).click()
  await expect(page.getByRole('status')).toHaveText('Email verified. You can now log in and use protected features.')
  expect(new URL(page.url()).searchParams.has('token')).toBe(false)
  await login(page, email, password)
  await expect(page.getByText('Signed in as @' + handle, { exact: true })).toBeVisible()

  const oldContext = await browser.newContext()
  try {
    const oldPage = await oldContext.newPage()
    await login(oldPage, email, password)
    await expect(oldPage.getByText('Signed in as @' + handle, { exact: true })).toBeVisible()
    await page.goto('/forgot-password')
    await page.getByLabel('Email', { exact: true }).fill(email)
    await page.getByRole('button', { name: 'Send reset link', exact: true }).click()
    await expect(page.getByRole('status')).toHaveText('If the address can receive account recovery email, a reset link has been queued.')
    const reset = await mailToken(request, email, '/reset-password')
    await openTokenPage(page, '/reset-password', reset)
    expect((await call(oldPage, 'GET', '/api/auth/session')).status).toBe(200)
    await page.getByLabel('New password', { exact: true }).fill(replacement)
    await page.getByLabel('Confirm new password', { exact: true }).fill(replacement)
    await page.getByRole('button', { name: 'Reset password', exact: true }).click()
    await expect(page.getByRole('status')).toHaveText('Password reset. Old sessions and API keys are revoked. Log in again with your new password.')
    expect(new URL(page.url()).searchParams.has('token')).toBe(false)
    expect((await call(oldPage, 'GET', '/api/auth/session')).status).toBe(401)
    await oldPage.reload()
    await expect(oldPage.getByText('Signed out', { exact: true })).toBeVisible()
    await login(oldPage, email, password)
    await expect(oldPage.getByRole('alert')).toHaveText('Email or password is incorrect.')
    await login(page, email, replacement)
    await expect(page.getByText('Signed in as @' + handle, { exact: true })).toBeVisible()
    await page.goto('/logout')
    expect((await call(page, 'GET', '/api/auth/session')).status).toBe(200)
    await page.getByRole('button', { name: 'Confirm logout', exact: true }).click()
    await expect(page.getByRole('status')).toHaveText('You are logged out.')
    await expect(page.getByText('Signed out', { exact: true })).toBeVisible()
    expect((await call(page, 'GET', '/api/auth/session')).status).toBe(401)
    // A reused link must show a safe failure, never a false completion screen.
    await openTokenPage(page, '/reset-password', reset)
    await page.getByLabel('New password', { exact: true }).fill(fixturePassword())
    await page.getByLabel('Confirm new password', { exact: true }).fill(await page.getByLabel('New password', { exact: true }).inputValue())
    await page.getByRole('button', { name: 'Reset password', exact: true }).click()
    await expect(page.getByRole('alert')).toHaveText('This link is invalid or has expired.')
    await expect(page.getByRole('link', { name: 'Request a fresh reset link' })).toBeVisible()
  } finally { await oldContext.close() }
})

test('account screens explain unverified restrictions, resend, and recover invalid links safely', async ({ page, request }) => {
  const handle = 'pending_' + randomUUID().replaceAll('-', '').slice(0, 12)
  const email = handle + '@example.test'
  const password = fixturePassword()
  await page.goto('/register')
  await page.getByLabel('Email', { exact: true }).fill(email)
  await page.getByLabel('Handle', { exact: true }).fill(handle)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Create account', exact: true }).click()
  await expect(page.getByRole('status')).toContainText('If registration can be completed')
  await login(page, email, password)
  await expect(page.getByText('Signed in as @' + handle + ' — email verification required', { exact: true })).toBeVisible()
  await page.getByRole('link', { name: 'Verify email', exact: true }).click()
  await page.getByRole('button', { name: 'Resend verification email' }).click()
  await expect(page.getByRole('status')).toHaveText('If verification is still needed, a new email has been queued.')
  // Confirm SMTP still works; the UI never pretends a token was consumed here.
  await mailToken(request, email, '/verify-email')
  await openTokenPage(page, '/verify-email', 'invalid-fixture-link')
  await page.getByRole('button', { name: 'Verify email', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveText('This link is invalid or has expired.')
  await expect(page.getByRole('link', { name: 'Start again with a fresh link' })).toBeVisible()
  await page.goto('/forgot-password')
  await page.getByLabel('Email', { exact: true }).fill('unknown-' + handle + '@example.test')
  await page.getByRole('button', { name: 'Send reset link' }).click()
  await expect(page.getByRole('status')).toHaveText('If the address can receive account recovery email, a reset link has been queued.')
})
