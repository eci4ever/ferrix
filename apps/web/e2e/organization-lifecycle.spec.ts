import { test, expect, type Page } from '@playwright/test'
async function workspace(page: Page, role = 'owner') {
  const queries: { path: string; q: URLSearchParams }[] = [], policies: unknown[] = []
  const members = Array.from({ length: 28 }, (_, n) => ({ id: `m${n}`, name: `Member ${n}`, email: `member${n}@example.com`, role: n === 0 ? 'owner' : 'member' }))
  let limit: number | null = null
  await page.route('**/api/v1/**', route => {
    const url = new URL(route.request().url()), path = url.pathname.replace('/api/v1', '')
    if (path.endsWith('/policy')) {
      if (route.request().method() === 'POST') { const body = route.request().postDataJSON(); policies.push(body); limit = body.member_limit; return route.fulfill({ status: 204 }) }
      return route.fulfill({ json: { member_limit: limit, invitation_limit: null } })
    }
    if (path === '/auth/members/search' || path === '/auth/invitations/search') {
      queries.push({ path, q: url.searchParams })
      const q = url.searchParams.get('q') ?? '', filterRole = url.searchParams.get('role'), status = url.searchParams.get('status')
      const all = path.includes('/members/') ? members : ['pending', 'accepted', 'expired', 'cancelled', 'rejected'].map(value => ({ id: value, email: `${value}@example.com`, role: 'member', status: value, inviter_name: 'Inviting owner', delivery_status: 'sent', expires_at: '2030-01-01T00:00:00Z' }))
      const filtered = all.filter(row => row.email.includes(q) && (!filterRole || row.role === filterRole) && (!status || ('status' in row && row.status === status)))
      const offset = Number(url.searchParams.get('page') ?? 0) * 25
      return route.fulfill({ json: { items: filtered.slice(offset, offset + 25), total: filtered.length } })
    }
    const user = { id: 'target', name: 'Target user', email: 'target@example.com', system_role: 'user', banned: false }
    const bodies: Record<string, unknown> = {
      '/auth/session': { user_id: 'owner', name: 'Owner', email: 'owner@example.com', system_role: 'admin', role, organization_id: 'org', organization_name: 'Lab' },
      '/auth/setup': { setup_required: false }, '/auth/profile': {}, '/auth/organizations': [],
      '/auth/organizations/current': { id: 'org', name: 'Lab', is_default: false, logo_url: null },
      '/status': { database: true, docker: true, version: 'test' }, '/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/auth/admin/users': { items: [user], total: 1 },
      '/auth/admin/users/target': { user, organizations: [{ id: 'org', name: 'Lab', role: 'member', joined_at: '2026-01-02T00:00:00Z' }], sessions: [], summary: { last_sign_in_at: '2026-11-28T00:00:00Z', mfa_enabled: true, api_keys: { active: 2, revoked: 1, expired: 3 } } },
      '/auth/activity': { items: [], total: 0, users: [{ id: 'target', name: 'Target user' }], organizations: [] },
    }
    if (path === '/auth/activity') queries.push({ path, q: url.searchParams })
    return route.fulfill({ json: bodies[path] ?? {} })
  })
  return { queries, policies }
}
test('Members and invitations use bounded server queries, filters and refresh', async ({ page }) => {
  const { queries } = await workspace(page)
  await page.goto('/organization/members')
  const table = page.getByRole('table', { name: 'Members', exact: true })
  await expect(table.getByRole('row')).toHaveCount(26)
  await page.getByRole('button', { name: 'Next Members page', exact: true }).click()
  await expect(table.getByRole('row')).toHaveCount(4)
  await page.getByLabel('Search Members', { exact: true }).fill('member27@')
  await expect(table.getByRole('row')).toHaveCount(2)
  await expect.poll(() => queries.filter(value => value.path.includes('/members/')).at(-1)?.q.get('page')).toBe('0')
  await page.getByLabel('Members role', { exact: true }).selectOption('owner')
  await expect(table).toContainText('No matching results.')
  await page.getByRole('tab', { name: 'Invitations', exact: true }).click()
  await page.getByLabel('Invitations status', { exact: true }).selectOption('rejected')
  const invitations = page.getByRole('table', { name: 'Invitations', exact: true })
  await expect(invitations).toContainText('rejected@example.com')
  await expect(invitations).toContainText('Inviting owner')
  await expect(invitations.getByRole('button', { name: 'Regenerate', exact: true })).toBeDisabled()
  await expect(invitations.getByRole('button', { name: 'Cancel invitation', exact: true })).toBeDisabled()
  const count = queries.filter(value => value.path.includes('/invitations/')).length
  await page.getByRole('button', { name: 'Refresh workspace', exact: true }).click()
  await expect.poll(() => queries.filter(value => value.path.includes('/invitations/')).length).toBeGreaterThan(count)
  await expect(page.getByLabel('Invitations status', { exact: true })).toHaveValue('rejected')
})
for (const role of ['owner', 'admin']) test(`Organization limits ${role} controls`, async ({ page }) => {
  const { policies } = await workspace(page, role)
  await page.goto('/organization/settings')
  const member = page.getByLabel('Member limit', { exact: true })
  if (role === 'owner') {
    await member.fill('40')
    await page.getByRole('button', { name: 'Save organization limits', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('Organization limits saved')
    expect(policies).toEqual([{ member_limit: 40, invitation_limit: null }])
    await page.getByRole('button', { name: 'Refresh workspace', exact: true }).click()
    await expect(member).toHaveValue('40')
  } else {
    await expect(member).toBeDisabled()
    await expect(page.getByRole('button', { name: 'Save organization limits', exact: true })).toHaveCount(0)
    expect(policies).toEqual([])
  }
  await expect(page.getByText('Require MFA', { exact: true })).toHaveCount(0)
})
test('User details show security metadata and link to filtered activity', async ({ page }) => {
  const { queries } = await workspace(page)
  await page.goto('/admin')
  await page.getByRole('button', { name: 'Details', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('MFA: Enabled')
  await expect(dialog).toContainText('2 active · 1 revoked · 3 expired')
  await expect(dialog.getByRole('table', { name: 'User organizations', exact: true })).toContainText('Joined')
  await dialog.getByRole('link', { name: 'View user activity', exact: true }).click()
  await expect.poll(() => queries.find(value => value.path === '/auth/activity')?.q.get('user_id')).toBe('target')
  await expect(page.getByLabel('User', { exact: true })).toHaveValue('target')
})

test('Ownership transfer can find a member beyond the first page', async ({ page }) => {
  await workspace(page)
  await page.goto('/organization/settings')
  await page.getByRole('tab', { name: 'Access', exact: true }).click()
  await page.getByLabel('Find member', { exact: true }).fill('member27@')
  await expect(page.getByLabel('New owner', { exact: true }).getByRole('option', { name: 'Member 27 (member27@example.com)', exact: true })).toHaveCount(1)
  await page.getByLabel('New owner', { exact: true }).selectOption('m27')
  await expect(page.getByRole('button', { name: 'Transfer ownership', exact: true })).toBeEnabled()
})
