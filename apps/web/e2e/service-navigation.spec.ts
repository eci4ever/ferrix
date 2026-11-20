import { test, expect } from '@playwright/test'

test('service breadcrumbs, tabs and runtime status remain clear', async ({ page }) => {
  const service = { id: 'service', project_id: 'project', environment_id: 'staging', kind: 'application', name: 'Web app', image: 'nginx:alpine', deployed: true, created_at: new Date().toISOString() }
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': { name: 'Admin', system_role: 'admin', role: 'owner', organization_id: 'org', organization_name: 'Team' },
      '/api/v1/auth/setup': { setup_required: false }, '/api/v1/auth/organizations': [], '/api/v1/status': { database: true, docker: true, version: 'test' },
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/api/v1/projects/project/environments': { project_name: 'Portal', environments: [{ id: 'prod', name: 'production' }, { id: 'staging', name: 'staging' }] },
      '/api/v1/services/service': service,
      '/api/v1/services/service/runtime': { state: 'running', health: 'healthy' },
      '/api/v1/services/service/deployments': [{ id: 'release', status: 'succeeded', created_at: service.created_at, logs: 'Done' }],
      '/api/v1/services/service/configuration': { name: 'Web app', image: 'nginx:alpine', container_port: 80, environment: '' },
      '/api/v1/services/service/environment': { environment: 'MODE=test' },
      '/api/v1/services/service/logs': { logs: 'Server ready' },
      '/api/v1/environments/staging/services': [service],
      '/api/v1/environments/staging/service-status': [{ id: 'service', state: 'running' }],
    }
    await route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/projects/project/services/service')
  const trail = page.getByRole('navigation', { name: 'Project navigation' })
  await expect(trail).toContainText('ProjectsPortalstagingWeb app')
  await expect(page.getByText(/Status checked/)).toBeVisible()
  await page.getByRole('tab', { name: 'Configuration', exact: true }).click()
  await page.getByRole('button', { name: 'Load configuration' }).click()
  await page.getByLabel('Application name').fill('Unsaved name')
  await page.getByRole('tab', { name: 'Deployments', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Deployments', exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Configuration', exact: true }).click()
  await expect(page.getByLabel('Application name')).toHaveValue('Unsaved name')
  await page.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByRole('tab', { name: 'Logs', exact: true }).click()
  await expect(page.getByText('Server ready', { exact: true })).toBeVisible()
  await trail.getByRole('link', { name: 'staging', exact: true }).click()
  await expect(page.getByRole('button', { name: 'staging', exact: true })).toBeVisible()
  await expect(page.getByRole('link', { name: 'Web app', exact: true })).toBeVisible()
  await page.setViewportSize({ width: 390, height: 844 })
  await page.getByRole('link', { name: 'Web app', exact: true }).click()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.route('**/api/v1/services/service/runtime', route => route.fulfill({ status: 503, json: { error: 'Docker unavailable' } }))
  await expect(page.getByText('Runtime status unavailable. Retrying automatically…', { exact: true })).toBeVisible()
})
