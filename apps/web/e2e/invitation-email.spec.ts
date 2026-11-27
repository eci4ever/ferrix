import { test, expect } from '@playwright/test'

for (const outcome of ['sent', 'failed', 'not_configured']) test(`Invitation email ${outcome} preserves create and regenerate tokens`, async ({ page }) => {
  let invitations: object[] = []
  let responseOutcome = outcome
  let delayed = outcome === 'failed'
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname.replace('/api/v1', '')
    if (path === '/auth/invitations' && route.request().method() === 'POST') {
      if (delayed) await new Promise(resolve => setTimeout(resolve, 11_000))
      invitations = [{ id: 'invite', email: 'invited@example.com', role: 'member', expires_at: '2030-01-01T00:00:00Z' }]
      return route.fulfill({ json: { token: 'created-token', email_delivery: responseOutcome } })
    }
    if (path === '/auth/invitations/invite/renew') {
      if (delayed) await new Promise(resolve => setTimeout(resolve, 11_000))
      return route.fulfill({ json: { token: 'renewed-token', email_delivery: responseOutcome } })
    }
    const responses: Record<string, unknown> = {
      '/auth/session': { user_id: 'owner', name: 'Owner', email: 'owner@example.com', system_role: 'user', role: 'owner', organization_id: 'org', organization_name: 'Lab' },
      '/auth/profile': { avatar: 'initials' }, '/auth/organizations': [], '/auth/setup': { setup_required: false },
      '/status': { database: true, docker: true, version: 'test' }, '/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: 'UTC+00:00' },
      '/auth/members': [], '/auth/invitations': invitations, '/auth/organizations/current': { id: 'org', name: 'Lab', logo_url: null, is_default: true },
    }
    return route.fulfill({ json: responses[path.replace(/\/search$/, '')] ?? {} })
  })
  await page.goto('/organization/members')
  await page.getByRole('tab', { name: 'Invitations', exact: true }).click()
  await page.getByRole('button', { name: 'Invite member', exact: true }).click()
  await page.getByLabel('Invitation email', { exact: true }).fill('invited@example.com')
  await page.getByRole('dialog').getByRole('button', { name: 'Invite member', exact: true }).click()
  const message = outcome === 'sent' ? 'Email accepted by the provider.' : outcome === 'failed' ? 'Email delivery could not be confirmed.' : 'Email is not configured.'
  await expect(page.getByRole('dialog')).toContainText(message)
  await expect(page.getByLabel('New invitation token', { exact: true })).toHaveText('created-token')
  await page.getByRole('button', { name: 'I saved the invitation token', exact: true }).click()
  await page.getByRole('button', { name: 'Regenerate', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click()
  const alert = page.getByRole('alert').filter({ hasText: 'Invitation token' })
  await expect(alert).toContainText(message)
  await expect(alert.locator('code')).toHaveText('renewed-token')
  delayed = false
  responseOutcome = outcome === 'sent' ? 'failed' : 'sent'
  await page.getByRole('button', { name: 'Invite member', exact: true }).click()
  await page.getByLabel('Invitation email', { exact: true }).fill('another@example.com')
  await page.getByRole('dialog').getByRole('button', { name: 'Invite member', exact: true }).click()
  await expect(page.getByRole('dialog')).toContainText(responseOutcome === 'sent' ? 'Email accepted by the provider.' : 'Email delivery could not be confirmed.')
  await page.getByRole('button', { name: 'I saved the invitation token', exact: true }).click()
  await expect(alert).toContainText(message)
  await expect(alert.locator('code')).toHaveText('renewed-token')
})
