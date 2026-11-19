import { expect, test } from '@playwright/test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

test('create, publish and roll back a static website from a project', async ({ page }) => {
  const build = await mkdtemp(join(tmpdir(), 'ferrix-website-build-'))
  try {
    await writeFile(join(build, 'index.html'), 'Documentation')
    const createdAt = new Date().toISOString()
    const site = { id: 'website', project_id: 'project', name: 'Documentation', domain: null as string | null, active_release_id: null as string | null, created_at: createdAt }
    const releases: { id: string; bytes: number; created_at: string }[] = []
    await page.route('**/api/v1/**', async route => {
      const request = route.request()
      const path = new URL(request.url()).pathname
      const responses: Record<string, unknown> = {
        '/api/v1/auth/session': { name: 'Admin', system_role: 'admin', organization_id: 'org', organization_name: 'Test', role: 'owner' },
        '/api/v1/auth/organizations': [],
        '/api/v1/auth/setup': { setup_required: false },
        '/api/v1/status': { database: true, docker: true, version: 'test' },
        '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
        '/api/v1/projects/project/environments': { project_name: 'Project', environments: [{ id: 'env', name: 'production' }] },
        '/api/v1/environments/env/services': [],
        '/api/v1/environments/env/service-status': [],
        '/api/v1/environments/env/static-websites': [],
        '/api/v1/static-websites/website': { site, releases },
      }
      if (request.method() === 'POST') {
        const body = request.postDataJSON()
        switch (path) {
          case '/api/v1/environments/env/static-websites':
            expect(body).toEqual({ name: 'Documentation' })
            return route.fulfill({ status: 201, json: site })
          case '/api/v1/static-websites/website/domain':
            site.domain = body.domain
            return route.fulfill({ status: 204 })
          case '/api/v1/static-websites/website/publish':
            expect(body.files).toEqual([{ path: 'index.html', hex: Buffer.from('Documentation').toString('hex') }])
            site.active_release_id = `release-${releases.length + 1}`
            releases.unshift({ id: site.active_release_id, bytes: 13, created_at: createdAt })
            return route.fulfill({ status: 201, json: { id: site.active_release_id } })
          case '/api/v1/static-websites/website/rollback':
            expect(body.release_id).toBe('release-1')
            site.active_release_id = body.release_id
            return route.fulfill({ status: 204 })
        }
      }
      return route.fulfill({ json: responses[path] ?? {} })
    })
    await page.goto('/projects/project')
    await page.getByRole('button', { name: 'Create Service' }).click()
    await page.getByRole('menuitem', { name: 'Static Website' }).click()
    await page.getByLabel('Website name').fill('Documentation')
    await page.getByRole('button', { name: 'Create Static Website' }).click()
    await page.getByRole('link', { name: 'Documentation' }).click()
    await page.getByLabel('DNS hostname').fill('docs.example.test')
    await page.getByRole('button', { name: 'Save domain' }).click()
    await expect(page.getByText('Domain saved.')).toBeVisible()
    for (let i = 0; i < 2; i++) {
      await page.getByLabel('Build folder').setInputFiles(build)
      await page.getByRole('button', { name: 'Publish build' }).click()
      await expect(page.getByText('Build published.')).toBeVisible()
      await expect(page.getByRole('button', { name: 'Publish build' })).toBeEnabled()
    }
    await expect(page.getByRole('link', { name: 'Open docs.example.test' })).toHaveAttribute('href', 'https://docs.example.test/')
    await page.getByRole('button', { name: 'Roll back' }).click()
    await expect(page.getByText('Release activated.')).toBeVisible()
    await expect(page.getByRole('list').filter({ hasText: 'release-' }).getByRole('listitem').nth(1)).toContainText('Live')
  } finally {
    await rm(build, { recursive: true, force: true })
  }
})
