import { expect, test } from '@playwright/test'

test('Docker tabs load on demand and recover from a failed request', async ({ page }) => {
  let imagesUnavailable = true
  let imageRequests = 0
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/v1/docker/images') {
      imageRequests++
      return imagesUnavailable
        ? route.fulfill({ status: 503, json: { error: 'Docker unavailable.' } })
        : route.fulfill({ json: [{ id: 'sha256:abc', tags: 'postgres:17', size: 1048576 }] })
    }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': { name: 'Admin', system_role: 'admin', organization_id: 'org', organization_name: 'Test', role: 'owner' },
      '/api/v1/auth/organizations': [],
      '/api/v1/auth/setup': { setup_required: false },
      '/api/v1/status': { database: true, docker: true, version: 'test' },
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/api/v1/containers': [
        { id: 'one', name: 'running-db', image: 'postgres:17', state: 'running', status: 'Up' },
        { id: 'two', name: 'stopped-db', image: 'postgres:17', state: 'exited', status: 'Exited' },
      ],
      '/api/v1/docker/swarm': { state: 'inactive', manager: false },
      '/api/v1/docker/health': { connected: true, version: '28.0' },
    }
    return route.fulfill({ json: responses[path] ?? {} })
  })
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/containers')
  await expect(page.getByRole('tab')).toHaveCount(8)
  await expect(page.getByRole('cell', { name: 'running-db' })).toBeVisible()
  expect(imageRequests).toBe(0)
  await page.getByLabel('Container state').selectOption('exited')
  await expect(page.getByRole('cell', { name: 'running-db' })).toHaveCount(0)
  const containers = page.getByRole('tab', { name: 'Containers', exact: true })
  await containers.focus()
  await containers.press('ArrowRight')
  await expect(page.getByText('inactive', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Images', exact: true }).click()
  await expect(page.getByText('Images unavailable')).toBeVisible()
  imagesUnavailable = false
  await page.getByRole('button', { name: 'Refresh images' }).click()
  await expect(page.getByRole('cell', { name: 'postgres:17' })).toBeVisible()
  await page.getByRole('tab', { name: 'Health', exact: true }).click()
  await expect(page.getByText('28.0', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})
