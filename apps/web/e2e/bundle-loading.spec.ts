import { test, expect } from '@playwright/test'

test('production pages load on demand while navigation remains available', async ({ page }) => {
  test.skip(process.env.FERRIX_E2E_PRODUCTION !== '1', 'Requires production preview assets')
  const scripts: string[] = []
  const errors: string[] = []
  page.on('request', request => { if (request.resourceType() === 'script') scripts.push(request.url()) })
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname.replace('/api/v1', '')
    const responses: Record<string, unknown> = {
      '/auth/session': { user_id: 'admin', name: 'Bundle test', email: 'test@example.com', system_role: 'admin', role: 'owner', organization_id: 'org', organization_name: 'Lab' },
      '/auth/profile': { avatar: 'initials' }, '/auth/setup': { setup_required: false }, '/auth/organizations': [],
      '/status': { database: true, docker: true, version: 'test' }, '/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: 'UTC+00:00' },
      '/projects': [], '/home-summary': { projects: 0, services: 0, deployments_24h: 0, needs_attention: 0, attention: [] }, '/containers': [],
    }
    return route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/dashboard')
  await expect(page.getByRole('heading', { name: 'Welcome back, Bundle test' })).toBeVisible()
  expect(scripts.some(url => /\/(docker|project-services|auth)-[^/]+\.js/.test(url))).toBe(false)
  let release!: () => void
  const blocked = new Promise<void>(resolve => { release = resolve })
  await page.route(/\/assets\/docker-[^/]+\.js$/, async route => { await blocked; await route.continue() })
  try {
    await page.getByRole('link', { name: 'Docker', exact: true }).click()
    await expect(page.getByRole('status', { name: 'Loading workspace', exact: true })).toBeVisible()
    await expect(page.getByRole('navigation', { name: 'Main navigation' })).toBeVisible()
  } finally { release() }
  await expect(page.getByRole('heading', { name: 'Containers', exact: true })).toBeVisible()
  expect(scripts.some(url => /\/docker-[^/]+\.js/.test(url))).toBe(true)
  await page.getByRole('link', { name: 'Home', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Welcome back, Bundle test' })).toBeVisible()
  expect(errors).toEqual([])
})
