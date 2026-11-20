import { test, expect } from '@playwright/test'

test('Profile edits name and avatar through the account menu', async ({ page }) => {
  const profile = { name: 'Owner', email: 'owner@example.com', avatar: 'initials', avatar_url: null as string | null }
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/v1/auth/profile' && route.request().method() === 'POST') {
      Object.assign(profile, route.request().postDataJSON())
      return route.fulfill({ status: 204 })
    }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/profile': profile,
      '/api/v1/auth/session': { ...profile, system_role: 'admin', role: 'owner', organization_id: 'org', organization_name: 'Test' },
      '/api/v1/auth/organizations': [],
      '/api/v1/auth/setup': { setup_required: false },
      '/api/v1/status': { database: true, docker: true, version: 'test' },
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
    }
    return route.fulfill({ json: responses[path] ?? {} })
  })
  await page.route('https://example.com/avatar.png', route => route.fulfill({
    contentType: 'image/png',
    body: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l9sAAAAASUVORK5CYII=', 'base64'),
  }))
  await page.goto('/profile')
  await page.getByRole('button', { name: 'Account menu' }).click()
  const menu = page.getByRole('menu')
  await expect(menu.getByRole('menuitem', { name: 'Profile' })).toBeVisible()
  await expect(menu.getByText('Security and sessions')).toHaveCount(0)
  await expect(menu.getByText('Organization', { exact: true })).toHaveCount(0)
  await page.keyboard.press('Escape')
  await page.getByLabel('Name', { exact: true }).fill('New Name')
  await page.getByRole('radio', { name: 'Rocket' }).check()
  await page.getByRole('button', { name: 'Save profile' }).click()
  await expect(page.getByText('Profile saved.')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Account menu' })).toContainText('New Name')
  await expect(page.getByRole('button', { name: 'Account menu' })).toContainText('🚀')
  await page.getByLabel('Profile image URL').fill('https://example.com/avatar.png')
  await page.getByRole('button', { name: 'Save profile' }).click()
  await expect(page.getByRole('button', { name: 'Account menu' }).locator('img')).toHaveAttribute('src', 'https://example.com/avatar.png')
  await page.reload()
  await expect(page.getByLabel('Profile image URL')).toHaveValue('https://example.com/avatar.png')
  await expect(page.getByLabel('Name', { exact: true })).toHaveValue('New Name')
})
