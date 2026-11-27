import { test, expect, type Page } from '@playwright/test'

test.use({ trace: 'off', screenshot: 'off', video: 'off' })
async function workspace(page: Page, system = 'admin', role = 'owner') {
  let timezone = 'UTC'
  let orgs = [{ id: 'org', name: 'Default lab', logo_url: null as string | null, is_default: true, member_count: 1, project_count: 0 }, { id: 'other', name: 'Other lab', logo_url: null as string | null, is_default: false, member_count: 2, project_count: 0 }]
  const posted: { section: string; values: Record<string, unknown> }[] = []
  const config = {
    timezone: { revision: 0, timezone: '', effective_timezone: 'UTC' },
    email: { revision: 0, provider: 'smtp', resend_api_key_configured: false, host: '', port: 587, security: 'starttls', username: '', from_address: '', from_name: '', password_configured: false },
    social: { revision: 0, google_client_id: '', github_client_id: '', google_client_secret_configured: false, github_client_secret_configured: false },
    timezones: ['UTC', 'Asia/Kuala_Lumpur'],
  }
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname.replace('/api/v1', '')
    if (path.startsWith('/auth/admin/organizations') && route.request().method() === 'POST') {
      const body = route.request().postDataJSON()
      if (path === '/auth/admin/organizations') { orgs.push({ id: 'created', name: body.name, logo_url: null, is_default: false, member_count: 1, project_count: 0 }); return route.fulfill({ json: { id: 'created' } }) }
      const id = path.split('/')[4], org = orgs.find(value => value.id === id)!
      if (path.endsWith('/delete')) {
        if (body.confirmation !== org.name) return route.fulfill({ status: 422, json: { error: 'Enter the exact organization name to confirm deletion.' } })
        orgs = orgs.filter(value => value.id !== id)
      } else { org.name = body.name; org.logo_url = body.logo_url }
      return route.fulfill({ status: 204 })
    }
    if (path.startsWith('/auth/platform-settings/') && route.request().method() === 'POST') {
      const section = path.split('/').at(-1) as 'timezone' | 'email' | 'social'
      const body = route.request().postDataJSON()
      posted.push({ section, values: body.values })
      expect(body.revision).toBe(config[section].revision)
      for (const [key, value] of Object.entries(body.values)) {
        if (key.includes('secret') || key === 'password' || key === 'resend_api_key') Object.assign(config[section], { [`${key}_configured`]: Boolean(value) })
        else Object.assign(config[section], { [key]: value })
      }
      config[section].revision++
      if (section === 'timezone') { timezone = body.values.timezone || 'UTC'; config.timezone.effective_timezone = timezone }
      return route.fulfill({ status: 204 })
    }
    const responses: Record<string, unknown> = {
      '/auth/session': { user_id: 'test', name: 'Test', email: 'test@example.com', system_role: system, role, organization_id: 'org', organization_name: 'Lab' },
      '/auth/profile': { avatar: 'initials' }, '/auth/organizations': [], '/auth/setup': { setup_required: false },
      '/status': { database: true, docker: true, version: 'test' }, '/server-time': { unix_ms: Date.now(), timezone, utc_offset: timezone === 'UTC' ? 'UTC+00:00' : 'UTC+08:00' },
      '/auth/admin/organizations': orgs,
      '/auth/platform-settings': config, '/auth/members': [], '/auth/invitations': [], '/auth/organizations/current': { id: 'org', name: 'Lab', logo_url: null, is_default: true },
      '/monitoring': { host: 'Host', os: 'Test OS', samples: [], interval_seconds: 5, docker: {} },
      '/containers': [],
      '/docker/swarm': { state: 'active', manager: true, service_list: [{ id: 'a'.repeat(25), name: 'Example service', mode: 'Replicated', replicas: 1, version: 1 }], node_list: [] },
    }
    return route.fulfill({ json: responses[path.replace(/\/search$/, '')] ?? {} })
  })
  return posted
}

for (const [system, role] of [['admin','member'], ['user','owner'], ['user','admin'], ['user','member']]) test(`Sidebar separates platform ${system} and organization ${role} permissions`, async ({ page }) => {
  await workspace(page, system, role)
  await page.goto('/monitoring')
  const nav = page.getByRole('navigation', { name: 'Main navigation' })
  await expect(nav.getByRole('link', { name: 'Monitoring', exact: true })).toBeVisible()
  await expect(nav.getByRole('link', { name: 'Docker', exact: true })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Monitoring', exact: true })).toBeVisible()
  await expect(nav.getByRole('link', { name: 'Users', exact: true })).toHaveCount(system === 'admin' ? 1 : 0)
  await expect(nav.getByRole('link', { name: 'Organizations', exact: true })).toHaveCount(system === 'admin' ? 1 : 0)
  await expect(nav.getByRole('link', { name: 'Settings', exact: true })).toHaveCount(system === 'admin' ? 1 : 0)
  await expect(nav.getByRole('link', { name: 'Members', exact: true })).toHaveCount(role === 'member' ? 0 : 1)
  await expect(nav.getByRole('link', { name: 'Security', exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: 'Account menu', exact: true }).click()
  await expect(page.getByRole('menuitem', { name: 'Security', exact: true })).toBeVisible()
  await expect(page.getByRole('menuitem', { name: 'API Keys', exact: true })).toBeVisible()
  await page.keyboard.press('Escape')
  await nav.getByRole('link', { name: 'Docker', exact: true }).click()
  await page.getByRole('tab', { name: 'Swarm', exact: true }).click()
  await page.getByRole('tab', { name: 'Services', exact: true }).click()
  await expect(page.getByRole('table', { name: 'Swarm services', exact: true })).toContainText('Example service')
  await expect(page.getByRole('button', { name: 'Scale Example service', exact: true })).toHaveCount(system === 'admin' ? 1 : 0)
  await expect(page.getByRole('button', { name: 'Restart Example service', exact: true })).toHaveCount(system === 'admin' ? 1 : 0)
  await expect(page.getByRole('button', { name: 'View logs for Example service', exact: true })).toHaveCount(system === 'admin' ? 1 : 0)
  if (system !== 'admin') {
    await page.goto('/platform/settings')
    await expect(page.getByText('Platform administrator access required', { exact: true })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Save timezone', exact: true })).toHaveCount(0)
  }
})

test('Platform settings saves timezone and configuration without exposing saved secrets', async ({ page }) => {
  const posted = await workspace(page)
  await page.goto('/platform/settings')
  await page.getByLabel('Timezone', { exact: true }).selectOption('Asia/Kuala_Lumpur')
  await page.getByRole('button', { name: 'Save timezone', exact: true }).click()
  await expect(page.getByText('Current effective timezone: Asia/Kuala_Lumpur', { exact: true })).toBeVisible()
  await expect(page.getByLabel('Server time · Asia/Kuala_Lumpur · UTC+08:00')).toBeVisible()
  await page.getByRole('tab', { name: 'Email', exact: true }).click()
  await expect(page.getByText(/to send new and regenerated invitations/)).toBeVisible()
  await page.getByLabel('SMTP host', { exact: true }).fill('smtp.example.com')
  await page.getByLabel('SMTP password', { exact: true }).fill('disposable-secret')
  await page.getByRole('button', { name: 'Save email settings', exact: true }).click()
  await expect(page.getByLabel('SMTP password', { exact: true })).toHaveValue('')
  await expect(page.getByText('A secret is saved. Leave blank to keep it, or enter a replacement.', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Save email settings', exact: true }).click()
  await expect.poll(() => posted.filter(value => value.section === 'email').length).toBe(2)
  expect(posted.at(-1)?.values).not.toHaveProperty('password')
  await page.getByLabel('Remove saved smtp password', { exact: true }).check()
  await page.getByRole('button', { name: 'Save email settings', exact: true }).click()
  await expect(page.getByText('No secret is saved. Enter a value to store it securely.', { exact: true })).toBeVisible()
  expect(posted.at(-1)?.values.password).toBe('')
  await page.getByLabel('Email provider', { exact: true }).selectOption('google')
  await page.getByLabel('Sender email', { exact: true }).fill('test@gmail.com')
  await page.getByLabel('Google app password', { exact: true }).fill('disposable-google-app')
  await page.getByRole('button', { name: 'Save email settings', exact: true }).click()
  await expect(page.getByLabel('Google app password', { exact: true })).toHaveValue('')
  expect(posted.at(-1)?.values.provider).toBe('google')
  await page.getByLabel('Email provider', { exact: true }).selectOption('resend')
  await page.getByLabel('Resend API key', { exact: true }).fill('re_disposable')
  await page.getByRole('button', { name: 'Save email settings', exact: true }).click()
  await expect(page.getByLabel('Resend API key', { exact: true })).toHaveValue('')
  await expect(page.getByLabel('Remove saved resend api key', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Save email settings', exact: true }).click()
  await expect.poll(() => posted.filter(value => value.section === 'email').length).toBe(6)
  expect(posted.at(-1)?.values).not.toHaveProperty('resend_api_key')
  await page.getByLabel('Remove saved resend api key', { exact: true }).check()
  await page.getByRole('button', { name: 'Save email settings', exact: true }).click()
  await expect(page.getByLabel('Remove saved resend api key', { exact: true })).toHaveCount(0)
  await page.getByRole('tab', { name: 'Social Login', exact: true }).click()
  await expect(page.getByText(/OAuth is not active yet/)).toBeVisible()
  await page.getByLabel('Google client ID', { exact: true }).fill('google-client')
  await page.getByLabel('Google client secret', { exact: true }).fill('disposable-google')
  await page.getByRole('button', { name: 'Save social login settings', exact: true }).click()
  await expect(page.getByLabel('Google client secret', { exact: true })).toHaveValue('')
  await page.getByRole('button', { name: 'Refresh workspace', exact: true }).click()
  await expect(page.getByRole('tab', { name: 'Social Login', exact: true })).toHaveAttribute('aria-selected', 'true')
  for (const width of [320, 390, 1440]) {
    await page.setViewportSize({ width, height: 900 })
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  }
})

test('Organization navigation separates membership and settings tabs', async ({ page }) => {
  await workspace(page, 'user', 'owner')
  await page.goto('/organization/members')
  await expect(page.getByRole('tab', { name: 'Members', exact: true })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'Invitations', exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Invitations', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Invite member', exact: true })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'General', exact: true })).toHaveCount(0)
  await page.getByRole('link', { name: 'Overview', exact: true }).click()
  await expect(page.getByRole('tab', { name: 'General', exact: true })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'Access', exact: true })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'Invitations', exact: true })).toHaveCount(0)
})


test('Platform administrators create, edit and delete organizations with confirmation', async ({ page }) => {
  await workspace(page)
  await page.goto('/platform/organizations')
  const table = page.getByRole('table', { name: 'Platform organizations', exact: true })
  await expect(table).toContainText('Other lab')
  await expect(page.getByRole('button', { name: 'Delete Default lab', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: 'Edit Other lab', exact: true }).click()
  let dialog = page.getByRole('dialog', { name: 'Edit organization', exact: true })
  await dialog.getByLabel('Organization name', { exact: true }).fill('Renamed lab')
  await dialog.getByRole('button', { name: 'Save organization', exact: true }).click()
  await expect(table).toContainText('Renamed lab')
  await page.getByRole('button', { name: 'Delete Renamed lab', exact: true }).click()
  dialog = page.getByRole('dialog', { name: 'Delete organization?', exact: true })
  await dialog.getByLabel('Type Renamed lab to confirm', { exact: true }).fill('wrong')
  await dialog.getByRole('button', { name: 'Delete organization', exact: true }).click()
  await expect(dialog.getByText('Enter the exact organization name to confirm deletion.', { exact: true })).toBeVisible()
  await dialog.getByLabel('Type Renamed lab to confirm', { exact: true }).fill('Renamed lab')
  await dialog.getByRole('button', { name: 'Delete organization', exact: true }).click()
  await expect(table).not.toContainText('Renamed lab')
  await page.getByRole('button', { name: 'Create organization', exact: true }).click()
  dialog = page.getByRole('dialog', { name: 'Create organization', exact: true })
  await dialog.getByLabel('Organization name', { exact: true }).fill('Created lab')
  await dialog.getByRole('button', { name: 'Create organization', exact: true }).click()
  await expect(table).toContainText('Created lab')
  await page.setViewportSize({ width: 320, height: 900 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})
