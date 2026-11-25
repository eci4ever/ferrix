import { test, expect, type Page } from '@playwright/test'

async function workspace(page: Page, options: { member?: boolean; failProject?: boolean; holdProject?: Promise<void> } = {}) {
  const now = new Date().toISOString()
  const projects: { id: string; name: string; description: string; created_at: string; service_count: number }[] = []
  const environments = [{ id: 'production', name: 'production' }]
  const services: Record<string, unknown>[] = []
  const organizations = [{ id: 'org', name: 'Test organization', role: 'owner' }]
  const invitations: Record<string, unknown>[] = []
  let failProject = Boolean(options.failProject)
  await page.route('**/api/v1/**', async route => {
    const req = route.request(), path = new URL(req.url()).pathname
    if (req.method() === 'POST') {
      const body = req.postDataJSON()
      if (path === '/api/v1/projects') {
        await options.holdProject
        if (failProject) { failProject = false; return route.fulfill({ status: 409, json: { error: 'This project name is taken. Choose a different name.' } }) }
        const project = { id: 'project', name: body.name, description: body.description, created_at: now, service_count: 0 }
        projects.push(project); return route.fulfill({ status: 201, json: project })
      }
      if (path === '/api/v1/projects/project/environments') { const environment = { id: 'staging', name: body.name }; environments.push(environment); return route.fulfill({ status: 201, json: environment }) }
      if (path.endsWith('/services')) { const service = { ...body, id: 'web', environment_id: 'staging', project_id: 'project', created_at: now, deployed: false }; services.push(service); return route.fulfill({ status: 201, json: service }) }
      if (path === '/api/v1/auth/organizations') { organizations.push({ id: 'new-org', name: body.name, role: 'owner' }); return route.fulfill({ json: organizations[1] }) }
      if (path === '/api/v1/auth/invitations') { invitations.push({ id: 'invite', email: body.email, role: body.role, expires_at: now }); return route.fulfill({ json: { token: 'mock-invitation-token' } }) }
    }
    if (req.method() === 'DELETE' && path === '/api/v1/services/web') { expect(req.postDataJSON()).toEqual({ name: 'Nginx', remove_data: false }); services.length = 0; return route.fulfill({ status: 204 }) }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': { user_id: 'user', name: 'Dialog test', email: 'dialog@example.com', system_role: options.member ? 'user' : 'admin', role: options.member ? 'member' : 'owner', organization_id: 'org', organization_name: 'Test organization' },
      '/api/v1/auth/setup': { setup_required: false }, '/api/v1/auth/organizations': organizations,
      '/api/v1/auth/organizations/current': { id: 'org', name: 'Test organization', logo_url: null, is_default: true },
      '/api/v1/auth/members': [], '/api/v1/auth/invitations': invitations,
      '/api/v1/status': { database: true, docker: true, version: 'test' }, '/api/v1/projects': projects,
      '/api/v1/projects/service-status': { checked_at: now, projects: [] },
      '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/api/v1/projects/project/environments': { project_name: 'Portal', environments },
      '/api/v1/environments/production/services': services, '/api/v1/environments/staging/services': services,
      '/api/v1/environments/production/service-status': [], '/api/v1/environments/staging/service-status': [],
    }
    return route.fulfill({ json: responses[path] ?? {} })
  })
}

for (const theme of ['light', 'dark']) for (const width of [320, 1440]) {
  test(`Project dialog keyboard, failed save and layout: ${theme}, ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 })
    await page.emulateMedia({ reducedMotion: 'reduce' })
    await page.addInitScript(theme => localStorage.setItem('ferrix-theme', theme), theme)
    let release!: () => void
    const held = new Promise<void>(resolve => { release = resolve })
    await workspace(page, { failProject: true, holdProject: held })
    await page.goto('/projects')
    const trigger = page.getByRole('button', { name: 'New project', exact: true })
    await trigger.focus(); await page.keyboard.press('Enter')
    const dialog = page.getByRole('dialog', { name: 'Create a project', exact: true })
    const name = dialog.getByLabel('Project name', { exact: true })
    await expect(name).toBeFocused()
    if (width === 320) expect(await name.evaluate(node => parseFloat(getComputedStyle(node).fontSize))).toBeGreaterThanOrEqual(16)
    expect(await dialog.evaluate(node => getComputedStyle(node).animationName)).toBe('none')
    expect(await dialog.evaluate(node => getComputedStyle(node).overscrollBehaviorY)).toBe('contain')
    for (let i = 0; i < 7; i++) { await page.keyboard.press('Tab'); expect(await dialog.evaluate(node => node.contains(document.activeElement))).toBe(true) }
    await page.keyboard.press('Escape'); await expect(trigger).toBeFocused()
    const emptyTrigger = page.getByRole('button', { name: 'Create your first project', exact: true })
    await emptyTrigger.click(); await page.keyboard.press('Escape'); await expect(emptyTrigger).toBeFocused()
    await trigger.click()
    await name.fill('Portal')
    const submit = dialog.getByRole('button', { name: 'Create project', exact: true })
    const contrast = async (selector: typeof dialog) => selector.evaluate(node => {
      const style = getComputedStyle(node), canvas = document.createElement('canvas'), ctx = canvas.getContext('2d')!
      const luminance = (color: string) => { ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = color; ctx.fillRect(0, 0, 1, 1); return [...ctx.getImageData(0, 0, 1, 1).data].slice(0, 3).map(v => { const c = v / 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4 }).reduce((a, c, i) => a + c * [0.2126, 0.7152, 0.0722][i], 0) }
      const fg = luminance(style.color), bg = luminance(style.backgroundColor)
      return (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05)
    })
    const bodyRatio = await contrast(dialog), buttonRatio = await contrast(submit)
    expect(bodyRatio).toBeGreaterThanOrEqual(4.5); expect(buttonRatio).toBeGreaterThanOrEqual(4.5)
    await test.info().attach('contrast', { body: JSON.stringify({ theme, width, bodyRatio, buttonRatio }), contentType: 'application/json' })
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    if (width === 320 && theme === 'dark') await page.screenshot({ path: test.info().outputPath('new-project-dialog.png'), fullPage: true, animations: 'disabled' })
    await submit.click()
    await expect(dialog.getByRole('button', { name: 'Creating…' })).toBeDisabled()
    await page.keyboard.press('Escape'); await expect(dialog).toBeVisible()
    release()
    await expect(dialog.getByRole('alert')).toContainText('Choose a different name')
    await expect(name).toHaveValue('Portal')
    await name.fill('Portal retry'); await dialog.getByRole('button', { name: 'Create project', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    await expect(page.getByRole('link', { name: 'Portal retry', exact: true })).toBeVisible()
  })
}

test('Environment, Application and service deletion stay in focused dialogs at 320px', async ({ page }) => {
  await workspace(page); await page.setViewportSize({ width: 320, height: 800 })
  await page.goto('/projects/project')
  const environmentTrigger = page.getByRole('button', { name: 'Project Environment', exact: true })
  await environmentTrigger.click()
  let dialog = page.getByRole('dialog', { name: 'Create environment', exact: true })
  await expect(dialog.getByLabel('Environment name')).toBeFocused()
  await page.keyboard.press('Escape'); await expect(environmentTrigger).toBeFocused()
  await environmentTrigger.click(); await dialog.getByLabel('Environment name').fill('staging')
  await dialog.getByRole('button', { name: 'Create environment', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await page.getByRole('button', { name: 'Create Service', exact: true }).focus()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('menuitem', { name: 'Application', exact: true })).toBeFocused()
  await page.keyboard.press('Enter')
  dialog = page.getByRole('dialog', { name: 'Create Application', exact: true })
  await expect(dialog.getByLabel('Application name', { exact: true })).toBeFocused()
  await dialog.getByLabel('Application name', { exact: true }).fill('Nginx')
  await dialog.getByRole('button', { name: 'Create Application', exact: true }).scrollIntoViewIfNeeded()
  await expect(dialog.getByRole('button', { name: 'Create Application', exact: true })).toBeInViewport()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.screenshot({ path: test.info().outputPath('new-application-dialog.png'), fullPage: false, animations: 'disabled' })
  await dialog.getByRole('button', { name: 'Create Application', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  const deleteTrigger = page.getByRole('button', { name: 'Delete Nginx', exact: true })
  await deleteTrigger.click(); await page.keyboard.press('Escape'); await expect(deleteTrigger).toBeFocused()
  await deleteTrigger.click(); dialog = page.getByRole('dialog', { name: 'Delete Nginx?', exact: true })
  await expect(dialog.getByRole('button', { name: 'Delete service', exact: true })).toBeDisabled()
  await dialog.getByLabel('Type the service name to confirm').fill('Nginx')
  await dialog.getByRole('button', { name: 'Delete service', exact: true }).click()
  await expect(dialog).toHaveCount(0)
})

test('Organization creation and invitation token remain in their dialogs', async ({ page }) => {
  await workspace(page); await page.goto('/organization')
  const trigger = page.getByRole('button', { name: 'New organization', exact: true })
  await trigger.click()
  let dialog = page.getByRole('dialog', { name: 'Create organization', exact: true })
  await dialog.getByLabel('New organization name').fill('Lab')
  await dialog.getByRole('button', { name: 'Create organization', exact: true }).click()
  await expect(dialog).toHaveCount(0); await expect(trigger).toBeFocused()
  await page.getByRole('button', { name: 'Invite member', exact: true }).click()
  dialog = page.getByRole('dialog', { name: 'Invite member', exact: true })
  await dialog.getByLabel('Invitation email').fill('member@example.com')
  await dialog.getByRole('button', { name: 'Invite member', exact: true }).click()
  dialog = page.getByRole('dialog', { name: 'Save invitation token', exact: true })
  await expect(dialog.locator('code')).toHaveText('mock-invitation-token')
  await expect(dialog.getByRole('button', { name: 'I saved the invitation token' })).toBeFocused()
  await page.keyboard.press('Escape'); await expect(dialog).toBeVisible()
  await dialog.getByRole('button', { name: 'I saved the invitation token' }).click()
  await expect(dialog).toHaveCount(0)
})

test('Members do not get manager-only creation actions', async ({ page }) => {
  await workspace(page, { member: true }); await page.goto('/projects')
  await expect(page.getByRole('heading', { name: 'Projects', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'New project', exact: true })).toHaveCount(0)
  await page.goto('/projects/project')
  await expect(page.getByRole('button', { name: 'Create Service', exact: true })).toHaveCount(0)
  await page.goto('/organization')
  await expect(page.getByRole('heading', { name: 'Organization', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'New organization', exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Invite member', exact: true })).toHaveCount(0)
})

test('Creating an organization preserves an unfinished organization-profile draft', async ({ page }) => {
  await workspace(page); await page.goto('/organization')
  const profileName = page.getByLabel('Organization name', { exact: true })
  await expect(profileName).toHaveValue('Test organization')
  await profileName.fill('Draft organization name')
  await page.getByRole('button', { name: 'New organization', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Create organization', exact: true })
  await dialog.getByLabel('New organization name').fill('Other organization')
  const refreshed = page.waitForResponse(response => response.url().endsWith('/api/v1/auth/organizations/current') && response.request().method() === 'GET')
  await dialog.getByRole('button', { name: 'Create organization', exact: true }).click()
  await refreshed
  await expect(dialog).toHaveCount(0)
  await expect(profileName).toHaveValue('Draft organization name', { timeout: 2000 })
})
