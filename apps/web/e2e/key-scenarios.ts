import { expect, type Page } from '@playwright/test'

export async function keySettingsScenario(page: Page, account: { password: string }) {
  await page.setViewportSize({ width: 320, height: 740 })
  await page.goto('/settings/keys')
  await expect(page.getByRole('status').filter({ hasText: 'Key creation is unavailable' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Create key', exact: true })).toBeDisabled()
  await expect(page.getByRole('checkbox', { name: 'profiles.read', exact: true })).toBeChecked()
  await expect(page.getByRole('checkbox', { name: 'posts.read', exact: true })).toBeChecked()
  await expect(page.getByRole('checkbox', { name: 'feed.read', exact: true })).toBeChecked()
  await expect(page.getByRole('checkbox', { name: 'posts.write', exact: true })).not.toBeChecked()
  await expect(page.getByLabel('Expires in days', { exact: true })).toHaveValue('90')
  await expect(page.getByLabel('One-time API key', { exact: true })).toHaveCount(0)
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)

  // This is solely a UI transport fixture, not an application/provider bypass.
  // Real management/persistence/recovery contracts have separate Rust tests.
  const metadata: Array<Record<string, unknown>> = []
  let creations = 0
  const secret = 'tc_ui_fixture_not_a_real_key_one_time_only_1234'
  await page.route('**/api/auth/keys', async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({ status: 200, json: { integrationAvailable: true, items: metadata } })
    } else {
      const body = route.request().postDataJSON()
      expect(body.permissions).toEqual(['profiles.read', 'posts.read', 'feed.read'])
      expect(body.expiresInDays).toBe(90)
      const key = { keyId: 'key_ui_fixture_' + creations++, name: body.name,
        maskedHint: 'tc_…1234', permissions: body.permissions,
        createdAt: new Date().toISOString(), expiresAt: new Date(Date.now() + 86400000).toISOString(),
        revokedAt: null, providerRevokedAt: null, status: 'active' }
      metadata.push(key)
      await route.fulfill({ status: 201, json: { key: secret, metadata: key } })
    }
  })
  await page.route('**/api/auth/keys/key_ui_fixture_*', async (route) => {
    const key = metadata.find((key) => route.request().url().endsWith('/' + key.keyId))!
    key.revokedAt = new Date().toISOString()
    key.status = 'revoked_pending_cleanup'
    await route.fulfill({ status: 204, body: '' })
  })
  await page.reload()
  for (let i = 0; i < 2; i++) {
    await page.getByLabel('Current password', { exact: true }).fill(account.password)
    await page.getByRole('button', { name: 'Confirm password', exact: true }).focus()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('status').filter({ hasText: 'Password confirmed.' })).toBeVisible()
    await expect(page.getByLabel('Current password', { exact: true })).toHaveValue('')
    await page.getByLabel('Key name', { exact: true }).fill('UI fixture ' + i)
    await page.getByRole('button', { name: 'Create key', exact: true }).click()
    // Boolean assertion avoids putting even a fake secret in failure output.
    expect(await page.getByLabel('One-time API key', { exact: true }).inputValue() === secret).toBe(true)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    const persisted = await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))
    expect(persisted.includes(secret)).toBe(false)
    expect(persisted.includes(account.password)).toBe(false)
    if (i === 0) {
      await page.getByRole('button', { name: 'Copy key', exact: true }).click()
      await expect(page.getByRole('status').filter({ hasText: /Copied\.|Clipboard unavailable/ })).toBeVisible()
      await page.getByRole('button', { name: 'Dismiss secret', exact: true }).click()
    } else {
      await page.getByRole('link', { name: 'Home', exact: true }).click()
      await page.getByRole('link', { name: 'API keys', exact: true }).click()
    }
    await expect(page.getByLabel('One-time API key', { exact: true })).toHaveCount(0)
    expect((await page.content()).includes(secret)).toBe(false)
    await page.reload()
    await expect(page.getByLabel('One-time API key', { exact: true })).toHaveCount(0)
  }
  await page.getByRole('button', { name: 'Revoke UI fixture 0', exact: true }).click()
  await expect(page.getByRole('status').filter({ hasText: 'Status: revoked pending cleanup' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Revoke UI fixture 0', exact: true })).toHaveCount(0)
  await page.unroute('**/api/auth/keys')
  await page.unroute('**/api/auth/keys/key_ui_fixture_*')
}
