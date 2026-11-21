import { test, expect } from '@playwright/test'

test('Tables sort, search and paginate while service cards remain direct links', async ({ page }) => {
  const errors: string[] = []
  const actions: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  const users = Array.from({ length: 25 }, (_, i) => ({ id: `user-${i}`, name: `User ${String(i).padStart(2, '0')}`, email: `user${i}@example.test`, system_role: 'user', banned: false }))
  const sessions = users.map((user, i) => ({ id: `session-${i}`, user_agent: user.name, ip_address: `127.0.0.${i + 1}`, created_at: '2026-11-01T00:00:00Z', expires_at: '2026-12-01T00:00:00Z', current: i === 0 }))
  const containers = users.map((user, i) => ({ id: user.id, name: `container-${String(i).padStart(2, '0')}`, image: 'nginx:alpine', state: 'running', status: 'Up one hour' }))
  const deployments = users.map((user, i) => ({ id: user.id, status: 'succeeded', created_at: new Date(Date.UTC(2026, 10, i + 1)).toISOString(), logs: `deployment-${i}-output` }))
  const service = { id: 'service', environment_id: 'environment', project_id: 'project', name: 'Clickable service', image: 'nginx:alpine', kind: 'application', deployed: true, created_at: '2026-11-01T00:00:00Z' }
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname.replace('/api/v1', '')
    if (route.request().method() === 'POST') { actions.push(path); return route.fulfill({ status: 204 }) }
    const bodies: Record<string, unknown> = {
      '/auth/session': { user_id: 'admin', name: 'Admin', email: 'admin@example.test', system_role: 'admin', organization_id: 'organization', organization_name: 'Test', role: 'owner' },
      '/auth/organizations': [{ id: 'organization', name: 'Test', role: 'owner' }], '/auth/setup': { setup_required: false },
      '/status': { database: true, docker: true, version: 'test' }, '/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/containers': containers, '/auth/admin/users': users, '/auth/sessions': sessions,
      '/auth/security': { two_factor_enabled: false, recovery_codes_remaining: 0 }, '/auth/audit': [],
      '/projects/project/environments': { project_name: 'Test project', environments: [{ id: 'environment', name: 'production' }] },
      '/environments/environment/services': [service], '/environments/environment/service-status': [{ id: 'service', state: 'running' }],
      '/services/service': service, '/services/service/runtime': { state: 'running', host_port: 8000 }, '/services/service/deployments': deployments,
    }
    return route.fulfill({ json: bodies[path] ?? [] })
  })
  await page.goto('/containers')
  const table = page.getByRole('table', { name: 'Containers', exact: true })
  await expect(table.getByRole('row')).toHaveCount(11)
  await page.getByRole('button', { name: 'Next Containers page', exact: true }).click()
  await expect(table.getByText('container-10', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Name', exact: true }).click()
  await expect(table.getByText('container-00', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Name', exact: true }).click()
  await expect(table.getByRole('row').nth(1)).toContainText('container-24')
  await page.getByRole('textbox', { name: 'Search Containers', exact: true }).fill('container-17')
  await expect(table.getByRole('row')).toHaveCount(2)
  await page.getByRole('textbox', { name: 'Search Containers', exact: true }).fill('nothing-matches')
  await expect(table.getByText('No matching results.')).toBeVisible()
  await page.getByRole('textbox', { name: 'Search Containers', exact: true }).fill('')
  await page.screenshot({ path: '/private/tmp/ferrix-containers-table.png', fullPage: true, animations: 'disabled' })
  await page.setViewportSize({ width: 390, height: 844 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto('/admin')
  await page.getByRole('textbox', { name: 'Search Users', exact: true }).fill('user24@example.test')
  await page.getByRole('button', { name: 'Ban', exact: true }).click()
  expect(actions).toContain('/auth/admin/users/user-24/ban')
  await page.goto('/security')
  await page.getByRole('textbox', { name: 'Search Sessions', exact: true }).fill('User 24')
  await page.getByRole('button', { name: 'Revoke', exact: true }).click()
  expect(actions).toContain('/auth/sessions/session-24/revoke')
  await page.goto('/projects/project')
  await expect(page.getByRole('link', { name: 'Open', exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: 'Delete Clickable service', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Delete Clickable service?', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Cancel', exact: true }).click()
  const card = page.locator('[data-slot="card"]').filter({ has: page.getByRole('link', { name: 'Clickable service', exact: true }) })
  await card.click({ position: { x: 100, y: 120 } })
  await expect(page).toHaveURL(/\/services\/service$/)
  await page.getByRole('tab', { name: 'Deployments', exact: true }).click()
  await page.getByRole('textbox', { name: 'Search Deployment history', exact: true }).fill('2026-11-25')
  await page.getByRole('button', { name: 'Progress', exact: true }).click()
  await expect(page.getByText('deployment-24-output', { exact: true })).toBeVisible()
  await page.setViewportSize({ width: 390, height: 844 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.screenshot({ path: '/private/tmp/ferrix-deployment-table-mobile.png', fullPage: true, animations: 'disabled' })
  expect(errors).toEqual([])
})
