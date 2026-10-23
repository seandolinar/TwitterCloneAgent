import { expect, type Browser, type Page } from '@playwright/test'
import { call } from './helpers'

export async function readSocialScenario(browser: Browser, alicePage: Page, bobPage: Page,
  alice: { handle: string; csrf: string }, bob: { handle: string; csrf: string }) {
  await alicePage.goto('/')
  await expect(alicePage.getByText('No posts yet.', { exact: true })).toBeVisible()
  const bobProfile = (await call(bobPage, 'GET', '/api/v1/me')).data
  for (let i = 0; i < 21; i++) {
    expect((await call(bobPage, 'POST', '/api/v1/posts', { body: `Bob paging fixture ${i}` }, bob.csrf)).status).toBe(201)
  }
  const alicePost = await call(alicePage, 'POST', '/api/v1/posts', { body: 'Alice own feed fixture' }, alice.csrf)
  expect(alicePost.status).toBe(201)
  await alicePage.goto('/')
  await expect(alicePage.getByRole('heading', { name: 'Home feed', exact: true })).toBeVisible()
  await expect(alicePage.locator('article')).toHaveCount(1)
  await expect(alicePage.getByText('Alice own feed fixture', { exact: true })).toBeVisible()
  expect((await call(alicePage, 'PUT', '/api/v1/me/following/' + bobProfile.id, undefined, alice.csrf)).status).toBe(204)
  await alicePage.reload()
  await expect(alicePage.locator('article')).toHaveCount(20)
  let failOnce = true
  await alicePage.route('**/api/v1/feed?*', async (route) => {
    if (new URL(route.request().url()).searchParams.has('cursor') && failOnce) {
      failOnce = false
      await route.fulfill({ status: 503, contentType: 'application/json',
        body: '{"error":{"code":"service_unavailable","message":"Paging fixture temporarily unavailable."}}' })
    } else await route.continue()
  })
  await alicePage.getByRole('button', { name: 'Load more posts', exact: true }).click()
  await expect(alicePage.getByRole('alert')).toHaveText('Paging fixture temporarily unavailable.')
  await expect(alicePage.locator('article')).toHaveCount(20)
  await alicePage.getByRole('button', { name: 'Try again', exact: true }).click()
  await expect(alicePage.locator('article')).toHaveCount(22)
  const ids = await alicePage.locator('article').evaluateAll((posts) => posts.map((post) => post.getAttribute('data-post-id')))
  expect(new Set(ids).size).toBe(22)
  await expect(alicePage.getByText('You’re all caught up.', { exact: true })).toBeVisible()
  await alicePage.unroute('**/api/v1/feed?*')

  await alicePage.goto('/settings/profile')
  const hostile = '<script>window.profileInjection = true</script>'
  await alicePage.getByLabel('Display name', { exact: true }).fill('Alice edited display')
  await alicePage.getByLabel('Biography', { exact: true }).fill(hostile)
  await alicePage.getByRole('button', { name: 'Save profile', exact: true }).click()
  await expect(alicePage.getByRole('status')).toHaveText('Profile saved.')
  await alicePage.getByRole('link', { name: 'View your profile', exact: true }).click()
  await expect(alicePage.getByRole('heading', { name: 'Alice edited display', exact: true })).toBeVisible()
  await expect(alicePage.getByText(hostile, { exact: true })).toBeVisible()
  expect(await alicePage.evaluate(() => 'profileInjection' in window)).toBe(false)

  const signedOut = await browser.newContext()
  try {
    const publicPage = await signedOut.newPage()
    await publicPage.goto('/profiles/' + alice.handle)
    await expect(publicPage.getByRole('heading', { name: 'Alice edited display', exact: true })).toBeVisible()
    await expect(publicPage.getByText('Alice own feed fixture', { exact: true })).toBeVisible()
    await expect(publicPage.getByRole('link', { name: 'Log in to follow and like', exact: true })).toBeVisible()
    await publicPage.goto('/posts/' + alicePost.data.id)
    await expect(publicPage.getByText('Alice own feed fixture', { exact: true })).toBeVisible()
    await publicPage.goto('/posts/00000000-0000-0000-0000-000000000001')
    await expect(publicPage.getByText('This profile or post was not found.', { exact: true })).toBeVisible()
    await publicPage.goto('/profiles/nobody_fixture_missing')
    await expect(publicPage.getByText('This profile or post was not found.', { exact: true })).toBeVisible()
  } finally { await signedOut.close() }
}

export async function writeSocialScenario(alicePage: Page, bobPage: Page,
  alice: { handle: string }, bob: { handle: string; csrf: string }) {
  await alicePage.setViewportSize({ width: 320, height: 740 })
  await alicePage.goto('/')
  const composer = alicePage.getByLabel('Post text', { exact: true })
  const publish = alicePage.getByRole('button', { name: 'Publish post', exact: true })
  await composer.fill(' \n\u0085')
  await expect(publish).toBeDisabled()
  await composer.fill('😀'.repeat(281))
  await expect(alicePage.getByText('281/280 Unicode code points', { exact: true })).toBeVisible()
  await expect(publish).toBeDisabled()
  await composer.fill('😀'.repeat(280))
  await expect(publish).toBeEnabled()
  const text = '<script>window.postInjection = true</script> 😀 social interaction'
  await composer.fill(text)
  await publish.focus()
  await expect(publish).toBeFocused()
  await alicePage.keyboard.press('Enter')
  await expect(alicePage.getByRole('status').filter({ hasText: 'Post published.' })).toBeVisible()
  await expect(composer).toHaveValue('')
  const card = alicePage.locator('article').filter({ hasText: text })
  await expect(card).toBeVisible()
  expect(await alicePage.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  expect(await alicePage.evaluate(() => 'postInjection' in window)).toBe(false)
  const id = await card.getAttribute('data-post-id')
  expect(id).toBeTruthy()

  await alicePage.goto('/profiles/' + bob.handle)
  await expect(alicePage.getByRole('button', { name: 'Unfollow', exact: true })).toBeVisible()
  await alicePage.getByRole('button', { name: 'Unfollow', exact: true }).focus()
  await alicePage.keyboard.press('Enter')
  await expect(alicePage.getByText('0 followers · 0 following · 21 posts', { exact: true })).toBeVisible()
  await alicePage.getByRole('button', { name: 'Follow', exact: true }).click()
  await expect(alicePage.getByText('1 followers · 0 following · 21 posts', { exact: true })).toBeVisible()

  await bobPage.goto('/posts/' + id)
  await expect(bobPage.getByRole('button', { name: 'Delete post', exact: true })).toHaveCount(0)
  const denied = await call(bobPage, 'DELETE', '/api/v1/posts/' + id, undefined, bob.csrf)
  expect(denied.status).toBe(403)
  await expect(bobPage.getByText(text, { exact: true })).toBeVisible()
  await bobPage.route('**/api/v1/posts/' + id + '/like', (route) => route.fulfill({
    status: 403, contentType: 'application/json',
    body: '{"error":{"code":"forbidden","message":"Rejected mutation fixture."}}',
  }))
  await bobPage.getByRole('button', { name: 'Like', exact: true }).click()
  await expect(bobPage.getByRole('alert')).toHaveText('Rejected mutation fixture.')
  await expect(bobPage.getByRole('button', { name: 'Like', exact: true })).toHaveAttribute('aria-pressed', 'false')
  await expect(bobPage.locator('article')).toContainText('0 likes')
  await bobPage.unroute('**/api/v1/posts/' + id + '/like')
  await bobPage.getByRole('button', { name: 'Like', exact: true }).click()
  await expect(bobPage.getByRole('button', { name: 'Unlike', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await expect(bobPage.locator('article')).toContainText('1 like')
  await bobPage.getByRole('button', { name: 'Unlike', exact: true }).click()
  await expect(bobPage.locator('article')).toContainText('0 likes')

  await alicePage.goto('/posts/' + id)
  await alicePage.getByRole('button', { name: 'Delete post', exact: true }).click()
  await alicePage.getByRole('button', { name: 'Keep post', exact: true }).click()
  await expect(alicePage.getByText(text, { exact: true })).toBeVisible()
  await alicePage.getByRole('button', { name: 'Delete post', exact: true }).click()
  await alicePage.getByRole('button', { name: 'Confirm delete post', exact: true }).click()
  await expect(alicePage.getByText('This profile or post was not found.', { exact: true })).toBeVisible()
}
