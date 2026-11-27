import { test, expect } from '@playwright/test'

test('workspace refresh reloads Users, user details and the organization profile', async ({ page }) => {
  let userName = 'Original user'
  let orgName = 'Original organization'
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname
    const user = { id: 'target', name: userName, email: 'target@example.com', system_role: 'user', banned: false, created_at: new Date().toISOString() }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': { user_id: 'admin', name: 'Admin', system_role: 'admin', role: 'owner', organization_id: 'org', organization_name: 'Test' },
      '/api/v1/auth/profile': { avatar: 'initials' },
      '/api/v1/auth/organizations': [],
      '/api/v1/auth/setup': { setup_required: false },
      '/api/v1/status': { database: true, docker: true, version: 'test' },
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/api/v1/auth/admin/users': { items: [user], total: 1 },
      '/api/v1/auth/admin/users/target': { user, organizations: [], sessions: [] },
      '/api/v1/auth/organizations/current': { id: 'org', name: orgName, logo_url: null, is_default: true },
      '/api/v1/auth/members': [],
      '/api/v1/auth/invitations': [],
    }
    return route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/admin')
  await expect(page.getByRole('table', { name: 'Users', exact: true })).toContainText('Original user')
  userName = 'Refreshed user'
  await page.getByRole('button', { name: 'Refresh workspace' }).click()
  await expect(page.getByRole('table', { name: 'Users', exact: true })).toContainText('Refreshed user')
  await page.getByRole('button', { name: 'Details', exact: true }).click()
  await expect(page.getByRole('dialog')).toContainText('Refreshed user')
  await page.getByRole('dialog').getByRole('button', { name: 'Close', exact: true }).click()
  await page.goto('/organization')
  await expect(page.getByLabel('Organization name', { exact: true })).toHaveValue(orgName)
  orgName = 'Refreshed organization'
  await page.getByRole('button', { name: 'Refresh workspace' }).click()
  await expect(page.getByLabel('Organization name', { exact: true })).toHaveValue(orgName)
})
