import { expect, test } from '@playwright/test'

test('Docker tabs load on demand and recover from a failed request', async ({ page }) => {
  let imagesUnavailable = true
  let imageRequests = 0
  let engineVersion = '28.0'
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/v1/docker/images') {
      imageRequests++
      return imagesUnavailable
        ? route.fulfill({ status: 503, json: { error: 'Docker unavailable.' } })
        : route.fulfill({ json: [{ id: 'sha256:abc', tags: 'postgres:17', size: 1048576 }] })
    }
    if (path === '/api/v1/docker/health') {
      return route.fulfill({ json: { connected: true, version: engineVersion } })
    }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': { name: 'Admin', system_role: 'admin', organization_id: 'org', organization_name: 'Test', role: 'owner' },
      '/api/v1/auth/organizations': [],
      '/api/v1/auth/setup': { setup_required: false },
      '/api/v1/status': { database: true, docker: true, version: '0.2.0' },
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/api/v1/containers': [
        { id: 'one', name: 'running-db', image: 'postgres:17', state: 'running', status: 'Up' },
        { id: 'two', name: 'stopped-db', image: 'postgres:17', state: 'exited', status: 'Exited' },
      ],
      '/api/v1/docker/swarm': { state: 'inactive', manager: false },
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
  const previousImageRequests = imageRequests
  engineVersion = '28.1'
  await page.getByRole('button', { name: 'Refresh workspace' }).click()
  await expect(page.getByText('28.1', { exact: true })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'Health', exact: true })).toHaveAttribute('aria-selected', 'true')
  expect(imageRequests).toBe(previousImageRequests)
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})

test('Swarm shows real node capacity, worker limits and inactive state without invented counts', async ({ page }) => {
  let swarm: Record<string, unknown> = { state: 'inactive', manager: false, node_list: null }
  let unavailable = false
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/v1/docker/swarm') return unavailable
      ? route.fulfill({ status: 503, json: { error: 'Docker unavailable.' } })
      : route.fulfill({ json: swarm })
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': { name: 'Admin', system_role: 'admin', organization_id: 'org', organization_name: 'Test', role: 'owner' },
      '/api/v1/auth/organizations': [], '/api/v1/auth/setup': { setup_required: false },
      '/api/v1/status': { database: true, docker: true, version: '0.2.0' },
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/api/v1/containers': [],
    }
    return route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/containers')
  await page.getByRole('tab', { name: 'Swarm', exact: true }).click()
  await expect(page.getByText('Version 0.2.0', { exact: true })).toBeVisible()
  await expect(page.getByText('Swarm is not active', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Nodes', exact: true }).click()
  await expect(page.getByText('Node inventory is unavailable', { exact: false })).toBeVisible()
  swarm = { state: 'active', manager: true, node_list: [
    { id: 'node-one', hostname: 'vps', state: 'ready', role: 'manager', leader: true, reachability: 'reachable', availability: 'active', engine_version: '28.5.0', nano_cpus: 2000000000, memory_bytes: 4294967296, address: '192.168.0.252' },
    { id: 'node-two', hostname: 'worker', state: 'down', role: 'worker', availability: 'drain' },
  ] }
  await page.getByRole('button', { name: 'Refresh swarm' }).click()
  await expect(page.getByRole('heading', { name: 'vps', exact: true })).toBeVisible()
  await expect(page.getByText('2 cores', { exact: true })).toBeVisible()
  await expect(page.getByText('4 GiB', { exact: true })).toBeVisible()
  await expect(page.getByText('Availability: drain', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Overview', exact: true }).click()
  await expect(page.getByText('1 / 2', { exact: true })).toBeVisible()
  await page.setViewportSize({ width: 390, height: 844 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  swarm = { state: 'active', manager: false, node_id: 'worker-id', node_list: null }
  await page.getByRole('button', { name: 'Refresh swarm' }).click()
  await expect(page.getByText('Manager access required', { exact: true })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'vps', exact: true })).toHaveCount(0)
  unavailable = true
  await page.getByRole('button', { name: 'Refresh swarm' }).click()
  await expect(page.getByText('Swarm unavailable', { exact: true })).toBeVisible()
  unavailable = false
  await page.getByRole('button', { name: 'Refresh swarm' }).click()
  await expect(page.getByText('Manager access required', { exact: true })).toBeVisible()
})
