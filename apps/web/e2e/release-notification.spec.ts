import { test, expect } from '@playwright/test'

for (const scenario of ['available', 'current', 'member', 'stale'] as const) {
  test(`Release sidebar: ${scenario}`, async ({ page }) => {
    let releaseRequests = 0
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await page.route('**/api/v1/**', route => {
      const path = new URL(route.request().url()).pathname
      if (path === '/api/v1/releases/latest') releaseRequests++
      const bodies: Record<string, unknown> = {
        '/api/v1/auth/session': { session_id: 's', user_id: 'u', name: 'Release test', email: 'release@example.com', system_role: scenario === 'member' ? 'user' : 'admin', organization_id: 'org', organization_name: 'Test', role: 'owner' },
        '/api/v1/auth/organizations': [{ id: 'org', name: 'Test', role: 'owner' }],
        '/api/v1/auth/setup': { setup_required: false },
        '/api/v1/status': { database: true, docker: true, version: '0.2.2' },
        '/api/v1/projects': [],
        '/api/v1/server-time': { unix_ms: Date.now(), timezone: 'UTC', utc_offset: '+00:00' },
        '/api/v1/releases/latest': { current_version: '0.2.2', latest_version: scenario === 'current' ? '0.2.2' : '0.2.3', update_available: scenario !== 'current', release_notes_url: 'https://github.com/eci4ever/ferrix/releases/tag/v0.2.3', upgrade_guide_url: 'https://github.com/eci4ever/ferrix/blob/v0.2.3/docs/production.md#operate-and-update', checked_at: '2026-11-24T12:00:00Z', check_failed: scenario === 'stale' },
        '/api/v1/home-summary': { projects: 0, projects_new_7d: 0, services: 0, applications: 0, databases: 0, deployments_24h: 0, succeeded_24h: 0, failed_24h: 0, in_progress_24h: 0, needs_attention: 0, attention: [] },
      }
      return route.fulfill({ json: bodies[path] ?? {} })
    })
    await page.goto('/dashboard')
    await expect(page.getByText('Version 0.2.2', { exact: true })).toBeVisible()
    const notice = page.getByRole('button', { name: 'Ferrix v0.2.3 available', exact: true })
    if (scenario === 'member' || scenario === 'current') {
      await expect(notice).toHaveCount(0)
      if (scenario === 'member') expect(releaseRequests).toBe(0)
      else expect(releaseRequests).toBeGreaterThan(0)
    } else {
      await notice.click()
      const dialog = page.getByRole('dialog', { name: 'Ferrix v0.2.3 is available' })
      await expect(dialog).toBeVisible()
      await expect(dialog).toContainText('You are running v0.2.2')
      await expect(dialog).toContainText('FERRIX_AUTH_KEY')
      await expect(dialog.getByRole('link', { name: 'Release notes' })).toHaveAttribute('href', 'https://github.com/eci4ever/ferrix/releases/tag/v0.2.3')
      await expect(dialog.getByRole('link', { name: 'Upgrade guide' })).toHaveAttribute('href', /v0\.2\.3\/docs\/production.md#operate-and-update$/)
      if (scenario === 'stale') await expect(dialog).toContainText('The latest check failed')
      else await expect(dialog).toContainText('Checked automatically every 6 hours')
      await page.setViewportSize({ width: 390, height: 844 })
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
      await page.keyboard.press('Escape')
      await expect(dialog).toHaveCount(0)
    }
    expect(errors).toEqual([])
  })
}
