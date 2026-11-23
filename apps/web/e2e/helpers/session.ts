import { readFile, writeFile } from 'node:fs/promises'
import { expect } from '@playwright/test'
import type { Page } from '@playwright/test'

// Each live suite can run first; no file relies on another suite creating its account.
export async function signInAdmin(page: Page) {
  if (process.env.FERRIX_E2E_DISPOSABLE !== '1') throw new Error('Use a disposable browser-test database.')
  const cache = process.env.FERRIX_E2E_SESSION_CACHE
  if (cache) {
    try {
      const cookies = JSON.parse(await readFile(cache, 'utf8'))
      await page.context().addCookies(cookies)
      const response = await page.request.get('/api/v1/auth/session')
      if (response.ok()) {
        const account = await response.json()
        if (account.email === 'services-ui@example.com' && account.system_role === 'admin') {
          await page.goto('/dashboard')
          await expect(page).toHaveURL(/\/dashboard$/)
          return
        }
      }
    } catch { /* A revoked session or an empty cache needs a real sign-in. */ }
  }
  const setup = await (await page.request.get('/api/v1/auth/setup')).json()
  await page.goto(setup.setup_required ? '/signup' : '/login')
  if (setup.setup_required) await page.getByLabel('Name', { exact: true }).fill('Services UI test')
  await page.getByLabel('Email', { exact: true }).fill('services-ui@example.com')
  await page.getByLabel('Password', { exact: true }).fill('A long disposable browser test password!')
  await expect(page.getByText('Use two-factor authentication', { exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: setup.setup_required ? 'Create account' : 'Login', exact: true }).click()
  await expect(page).toHaveURL(/\/dashboard$/)
  if (cache) await writeFile(cache, JSON.stringify(await page.context().cookies()), { mode: 0o600 })
}
