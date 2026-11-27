import { test, expect } from '@playwright/test'

// Never retain password, setup-key or recovery-code screenshots/traces.
test.use({ trace: 'off', screenshot: 'off', video: 'off' })

test('Security tabs preserve forms, session search and refresh while protecting MFA dialogs', async ({ page }) => {
  let enabled = false
  let events: { id: string; action: string; created_at: string }[] = []
  let sessions = [
    { id: 'current', user_agent: 'Current browser', ip_address: '127.0.0.1', current: true, expires_at: '2026-12-01T00:00:00Z' },
    { id: 'other', user_agent: 'Other browser', ip_address: '127.0.0.2', current: false, expires_at: '2026-12-01T00:00:00Z' },
  ]
  const actions: string[] = []
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname.replace('/api/v1', '')
    if (route.request().method() === 'POST') {
      actions.push(path)
      if (path === '/auth/mfa/enroll') return route.fulfill({ json: { secret: 'DISPOSABLEKEY', otpauth_url: 'otpauth://totp/Ferrix?secret=DISPOSABLEKEY' } })
      if (path === '/auth/mfa/confirm') { enabled = true; return route.fulfill({ json: { recovery_codes: ['disposable-code'] } }) }
      if (path === '/auth/sessions/other/revoke') sessions = sessions.filter(session => session.current)
      return route.fulfill({ status: 204 })
    }
    const responses: Record<string, unknown> = {
      '/auth/session': { user_id: 'admin', name: 'Admin', system_role: 'admin', role: 'owner', organization_id: 'org', organization_name: 'Test' },
      '/auth/profile': { avatar: 'initials' }, '/auth/organizations': [], '/auth/setup': { setup_required: false },
      '/status': { database: true, docker: true, version: 'test' }, '/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
      '/auth/sessions': sessions, '/auth/security': { two_factor_enabled: enabled, recovery_codes_remaining: enabled ? 1 : 0 }, '/auth/audit': events,
    }
    return route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/security')
  const verification = page.getByRole('tabpanel', { name: 'Verification', exact: true })
  await expect(verification.getByRole('heading', { name: 'Verify your identity' })).toBeVisible()
  await verification.getByLabel('Current password', { exact: true }).fill('Disposable verification password')
  await page.getByRole('tab', { name: 'Sessions', exact: true }).click()
  await expect(verification).toBeHidden()
  await page.getByLabel('Search Sessions', { exact: true }).fill('Other browser')
  await page.getByRole('tab', { name: 'Verification', exact: true }).click()
  await expect(verification.getByLabel('Current password', { exact: true })).toHaveValue('Disposable verification password')
  await verification.getByRole('button', { name: 'Verify identity', exact: true }).click()
  await expect(page.getByText('Identity verified', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Sessions', exact: true }).click()
  await expect(page.getByLabel('Search Sessions', { exact: true })).toHaveValue('Other browser')
  await page.getByRole('button', { name: 'Revoke', exact: true }).click()
  await expect(page.getByRole('table', { name: 'Sessions', exact: true })).not.toContainText('Other browser')
  expect(actions).toContain('/auth/sessions/other/revoke')
  await expect(page.getByRole('tab', { name: 'Sessions', exact: true })).toHaveAttribute('aria-selected', 'true')
  await page.getByRole('tab', { name: 'Two-factor', exact: true }).click()
  await page.getByRole('button', { name: 'Set up authenticator', exact: true }).click()
  let dialog = page.getByRole('dialog')
  await dialog.getByLabel('Confirm password', { exact: true }).fill('Disposable verification password')
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click()
  await dialog.getByLabel('Authenticator code', { exact: true }).fill('123456')
  await dialog.getByRole('button', { name: 'Enable two-factor authentication', exact: true }).click()
  dialog = page.getByRole('dialog', { name: 'Save your recovery codes', exact: true })
  await expect(dialog).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(dialog).toBeVisible()
  await dialog.getByRole('button', { name: 'I saved my codes', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Two-factor authentication', exact: true })).toBeFocused()
  await page.getByRole('tab', { name: 'Password', exact: true }).click()
  await expect(page.getByLabel('New password', { exact: true })).toBeVisible()
  await page.getByRole('tab', { name: 'Activity', exact: true }).click()
  await expect(page.getByText('No security activity yet.', { exact: true })).toBeVisible()
  events = [{ id: 'event', action: 'session.revoked', created_at: new Date().toISOString() }]
  await page.getByRole('button', { name: 'Refresh workspace', exact: true }).click()
  await expect(page.getByText('session.revoked', { exact: true })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'Activity', exact: true })).toHaveAttribute('aria-selected', 'true')
  await page.getByRole('tab', { name: 'Verification', exact: true }).focus()
  await page.keyboard.press('ArrowRight')
  await expect(page.getByRole('tab', { name: 'Sessions', exact: true })).toBeFocused()
  for (const width of [320, 390, 720, 1440]) {
    await page.setViewportSize({ width, height: 900 })
    await page.getByRole('tab', { name: 'Activity', exact: true }).click()
    await expect(page.getByRole('heading', { name: 'Organization security activity', exact: true })).toBeVisible()
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  }
})
