import { test, expect, type Page } from '@playwright/test'
async function fixture(page: Page, system = 'admin', role = 'member') {
  let name = 'Target lab'
  let members = [{ id: 'owner', name: 'Org owner', email: 'owner@example.com', role: 'owner', banned: false }, { id: 'member', name: 'Org member', email: 'member@example.com', role: 'member', banned: false }]
  let invitations: { id: string; email: string; role: string; expires_at: string; status?: string }[] = []
  const writes: { path: string; body: Record<string, string> }[] = [], activity: URLSearchParams[] = []
  await page.route('**/api/v1/**', route => {
    const url = new URL(route.request().url()), path = url.pathname.replace('/api/v1', '')
    if (path.startsWith('/auth/admin/organizations/target') && route.request().method() === 'POST') {
      const body = route.request().postDataJSON(); writes.push({ path, body })
      if (path.endsWith('/members/owner/remove')) return route.fulfill({ status: 422, json: { error: 'An organization must retain at least one owner.' } })
      if (path === '/auth/admin/organizations/target') name = body.name
      if (path.endsWith('/members/member/role')) members = members.map(member => member.id === 'member' ? { ...member, role: body.role } : member)
      if (path.endsWith('/members/member/remove')) members = members.filter(member => member.id !== 'member')
      if (path.endsWith('/invitations')) { invitations = [{ id: 'invite', email: body.email, role: body.role, expires_at: '2030-01-01T00:00:00Z' }]; return route.fulfill({ json: { token: 'created-token', email_delivery: 'not_configured' } }) }
      if (path.endsWith('/renew')) return route.fulfill({ json: { token: 'renewed-token', email_delivery: 'failed' } })
      if (path.endsWith('/revoke')) invitations = invitations.map(invite => ({ ...invite, status: 'cancelled' }))
      return route.fulfill({ status: 204 })
    }
    if (path === '/auth/activity') { activity.push(url.searchParams); return route.fulfill({ json: { items: [], total: 0, users: [], organizations: [] } }) }
    const bodies: Record<string, unknown> = {
      '/auth/session': { user_id: 'platform', name: 'Platform admin', email: 'platform@example.com', system_role: system, role, organization_id: 'active', organization_name: 'Active lab' },
      '/auth/profile': { avatar: 'initials' }, '/auth/setup': { setup_required: false }, '/auth/organizations': [{ id: 'active', name: 'Active lab', role }],
      '/status': { database: true, docker: true, version: 'test' }, '/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: 'UTC+00:00' },
      '/auth/admin/organizations': [{ id: 'target', name, logo_url: null, is_default: false, member_count: members.length, project_count: 0 }],
      '/auth/admin/organizations/target/members': { items: members, total: members.length },
      '/auth/admin/organizations/target/invitations': { items: invitations, total: invitations.length },
      '/auth/admin/organizations/target/policy': { member_limit: null, invitation_limit: null },
      '/auth/admin/organizations/target': { organization: { id: 'target', name, logo_url: null, is_default: false, project_count: 0, created_at: '2026-11-27T00:00:00Z' }, members, invitations },
    }
    return route.fulfill({ json: bodies[path.replace(/\/search$/, '')] ?? {} })
  })
  return { writes, activity }
}
test('Platform admin manages target organization without switching membership context', async ({ page }) => {
  const { writes, activity } = await fixture(page)
  await page.goto('/platform/organizations')
  await page.getByRole('link', { name: 'Target lab', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Target lab', exact: true })).toBeVisible()
  await page.getByLabel('Organization name', { exact: true }).fill('Renamed target')
  await page.getByRole('button', { name: 'Save organization', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Renamed target', exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Members', exact: true }).click()
  await page.getByRole('button', { name: 'Change role for member@example.com', exact: true }).click()
  await page.getByLabel('Role', { exact: true }).selectOption('admin')
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click()
  await expect(page.getByRole('row').filter({ hasText: 'member@example.com' })).toContainText('admin')
  await page.getByRole('button', { name: 'Remove owner@example.com', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click()
  await expect(page.getByRole('dialog')).toContainText('retain at least one owner')
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByRole('button', { name: 'Remove member@example.com', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click()
  await expect(page.getByRole('table', { name: 'Organization members', exact: true })).not.toContainText('member@example.com')
  await page.getByRole('tab', { name: 'Invitations', exact: true }).click()
  await page.getByRole('button', { name: 'Invite member', exact: true }).click()
  await page.getByLabel('Invitation email', { exact: true }).fill('invited@example.com')
  await page.getByLabel('Role', { exact: true }).selectOption('admin')
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click()
  const token = page.getByRole('alert').filter({ hasText: 'Invitation token' })
  await expect(token).toContainText('created-token')
  await page.getByRole('button', { name: 'Regenerate', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click()
  await expect(token).toContainText('renewed-token')
  await expect(token).toContainText('could not be confirmed')
  await page.getByRole('button', { name: 'Refresh workspace', exact: true }).click()
  await expect(page.getByRole('tab', { name: 'Invitations', exact: true })).toHaveAttribute('aria-selected', 'true')
  await expect(token).toContainText('renewed-token')
  await page.getByRole('button', { name: 'Cancel invitation', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click()
  await expect(page.getByRole('table', { name: 'Organization invitations', exact: true })).toContainText('cancelled')
  await expect(page.getByRole('button', { name: 'Regenerate', exact: true })).toBeDisabled()
  await page.getByRole('tab', { name: 'Activity', exact: true }).click()
  await expect.poll(() => activity.length).toBeGreaterThan(0)
  await page.getByLabel('Search Activity', { exact: true }).fill('platform.')
  await expect.poll(() => activity.at(-1)?.get('q')).toBe('platform.')
  expect(activity.every(query => query.get('organization_id') === 'target')).toBe(true)
  await expect(page.getByLabel('Organization', { exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Switch organization', exact: true })).toContainText('Active lab')
  expect(writes.every(value => value.path.startsWith('/auth/admin/organizations/target'))).toBe(true)
  await page.setViewportSize({ width: 390, height: 844 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})
for (const role of ['owner', 'admin', 'member']) test(`Organization ${role} cannot open platform detail`, async ({ page }) => {
  const { writes } = await fixture(page, 'user', role)
  await page.goto('/platform/organizations/target')
  await expect(page.getByText('Platform administrator access required', { exact: true })).toBeVisible()
  await expect(page.getByRole('tab')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Save organization', exact: true })).toHaveCount(0)
  expect(writes).toEqual([])
})
