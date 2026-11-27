import { test, expect } from '@playwright/test'

test('project deletion requires an exact name and blocks projects with services', async ({ page }) => {
  let projects = [{ id: 'empty', name: 'Empty project', description: '', service_count: 0, created_at: new Date().toISOString() }, { id: 'full', name: 'Keep project', description: '', service_count: 2, created_at: new Date().toISOString() }]
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (route.request().method() === 'DELETE') {
      expect(path).toBe('/api/v1/projects/empty')
      expect(route.request().postDataJSON()).toEqual({ name: 'Empty project' })
      projects = projects.filter(project => project.id !== 'empty')
      return route.fulfill({ status: 204 })
    }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': { name: 'Admin', system_role: 'admin', role: 'owner', organization_id: 'org', organization_name: 'Team' },
      '/api/v1/auth/setup': { setup_required: false }, '/api/v1/auth/organizations': [], '/api/v1/projects': projects,
      '/api/v1/projects/service-status': { checked_at: new Date().toISOString(), projects: [] },
      '/api/v1/status': { database: true, docker: true, version: 'test' },
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
    }
    await route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/projects')
  await page.getByRole('radio', { name: 'Card view', exact: true }).click()
  await page.getByRole('button', { name: 'Delete Keep project', exact: true }).click()
  const blocked = page.getByRole('alertdialog')
  await expect(blocked).toContainText('This project has 2 services')
  await expect(blocked.getByRole('button', { name: 'Delete project', exact: true })).toBeDisabled()
  await blocked.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByRole('radio', { name: 'Table view', exact: true }).click()
  await page.getByRole('button', { name: 'Delete Empty project', exact: true }).click()
  const dialog = page.getByRole('alertdialog')
  await dialog.getByLabel('Type the project name to confirm').fill('Empty project ')
  await expect(dialog.getByRole('button', { name: 'Delete project', exact: true })).toBeDisabled()
  await dialog.getByLabel('Type the project name to confirm').fill('Empty project')
  await dialog.getByRole('button', { name: 'Delete project', exact: true }).click()
  await expect(page.getByRole('link', { name: 'Empty project', exact: true })).toHaveCount(0)
  await expect(page.getByRole('link', { name: 'Keep project', exact: true })).toBeVisible()
  await expect(page.getByText('Project “Empty project” deleted.', { exact: true })).toBeVisible()
  await page.route('**/api/v1/auth/session', route => route.fulfill({ json: { name: 'Member', role: 'member', system_role: 'user', organization_id: 'org', organization_name: 'Team' } }))
  await page.reload()
  await expect(page.getByRole('link', { name: 'Keep project', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Delete Keep project', exact: true })).toHaveCount(0)
})
