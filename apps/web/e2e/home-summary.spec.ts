import { test, expect } from '@playwright/test'

test('Home shows summary totals, attention links, empty and failure states on mobile', async ({ page }) => {
  let failed = false
  let empty = false
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/v1/home-summary' && failed) return route.fulfill({ status: 503, json: { error: 'Database unavailable.' } })
    const bodies: Record<string, unknown> = {
      '/api/v1/auth/session': { session_id: 's', user_id: 'u', name: 'Home test', email: 'home@example.com', system_role: 'user', organization_id: 'org', organization_name: 'Test organization', role: 'member' },
      '/api/v1/auth/organizations': [{ id: 'org', name: 'Test organization', role: 'member' }],
      '/api/v1/auth/setup': { setup_required: false },
      '/api/v1/status': { database: true, docker: true, version: 'test' },
      '/api/v1/projects': [],
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/api/v1/home-summary': { projects: empty ? 0 : 102, projects_new_7d: 0, services: empty ? 0 : 8, applications: empty ? 0 : 4, databases: empty ? 0 : 3, compose: empty ? 0 : 1, deployments_24h: 0, succeeded_24h: 0, failed_24h: 0, in_progress_24h: 0, needs_attention: empty ? 0 : 1, attention: empty ? [] : [{ id: 'service', project_id: 'project', name: 'Failed service' }] },
    }
    return route.fulfill({ json: bodies[path] ?? {} })
  })
  await page.goto('/dashboard')
  await expect(page.getByText('102', { exact: true })).toBeVisible()
  await expect(page.getByText('4 applications · 3 databases · 1 Compose · 0 websites')).toBeVisible()
  await expect(page.getByRole('link', { name: 'Failed service', exact: true })).toHaveAttribute('href', '/projects/project/services/service')
  await page.setViewportSize({ width: 320, height: 900 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  empty = true
  await page.reload()
  await expect(page.getByText('No failed latest deployments')).toBeVisible()
  failed = true
  await page.reload()
  await expect(page.getByText('Database unavailable. Use Refresh to retry.')).toBeVisible()
  await expect(page.getByText('Unavailable', { exact: true })).toHaveCount(4)
  failed = false
  await page.getByRole('button', { name: 'Refresh workspace', exact: true }).click()
  await expect(page.getByText('No failed latest deployments')).toBeVisible()
  expect(errors).toEqual([])
})
