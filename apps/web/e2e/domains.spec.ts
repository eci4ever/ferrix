import { test, expect } from '@playwright/test'

test('Domains explains DNS, checks HTTPS and confirms removal on mobile', async ({
  page,
}) => {
  const now = new Date().toISOString()
  const service = {
    id: 'web',
    project_id: 'project',
    environment_id: 'production',
    kind: 'application',
    name: 'Web',
    image: 'nginx:alpine',
    deployed: true,
    created_at: now,
  }
  let domains: Array<Record<string, unknown>> = []
  let member = false
  await page.route('**/api/v1/**', async (route) => {
    const req = route.request()
    const path = new URL(req.url()).pathname
    const base = '/api/v1/services/web/domains'
    if (path === base) {
      if (req.method() === 'POST') {
        expect(req.postDataJSON()).toEqual({ hostname: 'app.example.com' })
        domains = [
          {
            id: 'domain',
            hostname: 'app.example.com',
            verification_token: 'example-public-dns-proof-token',
            verified_at: null,
            dns_status: 'pending',
            tls_status: 'pending',
            certificate_expires_at: null,
            checked_at: null,
            message: null,
          },
        ]
        await route.fulfill({ status: 201, json: { id: 'domain' } })
        return
      }
      await route.fulfill({
        json: {
          domains,
          target_ip: '203.0.113.10',
          automatic_url: 'https://app-generated.example.com/',
          https_enabled: true,
          local: false,
          port: 443,
        },
      })
      return
    }
    if (path === `${base}/domain/check`) {
      expect(req.method()).toBe('POST')
      domains = domains.map((domain) => ({
        ...domain,
        verified_at: now,
        dns_status: 'ready',
        tls_status: 'active',
        checked_at: now,
        certificate_expires_at: '2030-01-01T00:00:00Z',
        message: 'HTTPS verified with a trusted certificate.',
      }))
      await route.fulfill({ json: domains[0] })
      return
    }
    if (path === `${base}/domain`) {
      expect(req.method()).toBe('DELETE')
      domains = []
      await route.fulfill({ status: 204 })
      return
    }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': {
        name: 'Admin',
        system_role: 'admin',
        role: member ? 'member' : 'owner',
        organization_id: 'org',
        organization_name: 'Team',
      },
      '/api/v1/auth/setup': { setup_required: false },
      '/api/v1/auth/organizations': [],
      '/api/v1/status': { database: true, docker: true, version: 'test' },
      '/api/v1/server-time': {
        unix_ms: Date.now(),
        timezone: 'UTC',
        utc_offset: '+00:00',
      },
      '/api/v1/projects/project/environments': {
        project_name: 'Portal',
        environments: [{ id: 'production', name: 'production' }],
      },
      '/api/v1/services/web': service,
      '/api/v1/services/web/runtime': { state: 'running' },
      '/api/v1/services/web/deployments': [],
    }
    await route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/projects/project/services/web')
  await page.getByRole('tab', { name: 'Domains', exact: true }).click()
  await page.getByLabel('Domain name', { exact: true }).fill('app.example.com')
  await page.getByRole('button', { name: 'Add domain', exact: true }).click()
  await expect(page.getByText('Waiting for DNS', { exact: true })).toBeVisible()
  await expect(
    page.getByText('_ferrix-challenge.app.example.com', { exact: true }),
  ).toBeVisible()
  await expect(
    page.getByText('example-public-dns-proof-token', { exact: true }),
  ).toBeVisible()
  await page
    .getByRole('button', { name: 'Check DNS & HTTPS', exact: true })
    .click()
  await expect(page.getByText('HTTPS active', { exact: true })).toBeVisible()
  await expect(page.getByText(/Certificate expires/)).toBeVisible()
  await expect(
    page.getByRole('link', { name: 'Open website', exact: true }),
  ).toHaveAttribute('href', 'https://app.example.com/')
  await page.setViewportSize({ width: 390, height: 844 })
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true)
  await page.screenshot({
    path: '/private/tmp/ferrix-domains-mobile.png',
    fullPage: true,
    animations: 'disabled',
  })
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.screenshot({
    path: '/private/tmp/ferrix-domains-desktop.png',
    fullPage: true,
    animations: 'disabled',
  })
  member = true
  await page.reload()
  await page.getByRole('tab', { name: 'Domains', exact: true }).click()
  await expect(page.getByText('HTTPS active', { exact: true })).toBeVisible()
  await expect(
    page.getByRole('button', { name: 'Add domain', exact: true }),
  ).toHaveCount(0)
  await expect(
    page.getByRole('button', { name: 'Check DNS & HTTPS', exact: true }),
  ).toHaveCount(0)
  member = false
  await page.reload()
  await page.getByRole('tab', { name: 'Domains', exact: true }).click()
  await page
    .getByRole('button', { name: 'Remove app.example.com', exact: true })
    .click()
  const dialog = page.getByRole('dialog', {
    name: 'Remove domain?',
    exact: true,
  })
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
  await expect(
    page.getByRole('heading', { name: 'app.example.com', exact: true }),
  ).toBeVisible()
  await page
    .getByRole('button', { name: 'Remove app.example.com', exact: true })
    .click()
  await dialog
    .getByRole('button', { name: 'Remove domain', exact: true })
    .click()
  await expect(
    page.getByRole('heading', { name: 'app.example.com', exact: true }),
  ).toHaveCount(0)
})
