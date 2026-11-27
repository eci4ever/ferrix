import { test, expect } from '@playwright/test'

for (const [system, role] of [['admin', 'member'], ['user', 'owner'], ['user', 'admin'], ['user', 'member']]) test(`Activity filters and refresh for ${system}/${role}`, async ({ page }) => {
  const queries: URLSearchParams[] = []
  let action = 'profile.updated'
  let fail = false
  await page.route('**/api/v1/**', route => {
    const url = new URL(route.request().url()), path = url.pathname.replace('/api/v1', '')
    if (path === '/auth/activity') {
      queries.push(url.searchParams)
      if (fail) return route.fulfill({ status: 503, json: { error: 'Temporary audit failure' } })
      return route.fulfill({ json: { total: url.searchParams.get('q') === 'empty' ? 0 : 26, items: url.searchParams.get('q') === 'empty' ? [] : [{ id: 'event', user_name: 'Test actor', organization_name: 'Audit lab', action, outcome: 'succeeded', target_id: null, created_at: '2026-11-27T10:00:00Z' }], users: [{ id: 'actor', name: 'Test actor' }], organizations: [{ id: 'org', name: 'Audit lab' }] } })
    }
    const responses: Record<string, unknown> = {
      '/auth/session': { user_id: 'actor', name: 'Test actor', email: 'test@example.com', system_role: system, role, organization_id: 'org', organization_name: 'Audit lab' },
      '/auth/profile': { avatar: 'initials' }, '/auth/organizations': [], '/auth/setup': { setup_required: false },
      '/status': { database: true, docker: true, version: 'test' }, '/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: 'UTC+00:00' },
    }
    return route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/activity')
  const table = page.getByRole('table', { name: 'Activity', exact: true })
  await expect(table).toContainText(action)
  await expect(page.getByLabel('Organization', { exact: true })).toHaveCount(system === 'admin' ? 1 : 0)
  await expect(page.getByLabel('User', { exact: true })).toHaveCount(system === 'admin' || role !== 'member' ? 1 : 0)
  if (system === 'admin') {
    await page.getByLabel('Organization', { exact: true }).selectOption('org')
    await expect.poll(() => queries.at(-1)?.get('organization_id')).toBe('org')
  }
  if (system === 'admin' || role !== 'member') {
    await page.getByLabel('User', { exact: true }).selectOption('actor')
    await expect.poll(() => queries.at(-1)?.get('user_id')).toBe('actor')
  }
  await page.getByLabel('Search Activity', { exact: true }).fill('profile')
  await expect.poll(() => queries.at(-1)?.get('q')).toBe('profile')
  await expect(table).toContainText(action)
  await page.getByRole('button', { name: 'Next Activity page', exact: true }).click()
  await expect.poll(() => queries.at(-1)?.get('page')).toBe('1')
  action = 'organization.switched'
  await page.getByRole('button', { name: 'Refresh workspace', exact: true }).click()
  await expect(table).toContainText(action)
  await page.getByLabel('Search Activity', { exact: true }).fill('empty')
  await expect(table).toContainText('No matching results')
  await expect.poll(() => queries.at(-1)?.get('page')).toBe('0')
  fail = true
  await page.getByRole('button', { name: 'Refresh workspace', exact: true }).click()
  await expect(page.getByText('Activity could not be loaded', { exact: true })).toBeVisible()
  fail = false
  await page.getByRole('button', { name: 'Retry activity', exact: true }).click()
  await page.getByRole('button', { name: 'Clear filters', exact: true }).click()
  await expect(table).toContainText(action)
  await page.setViewportSize({ width: 390, height: 844 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})
