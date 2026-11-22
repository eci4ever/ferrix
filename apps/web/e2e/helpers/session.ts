import { expect } from '@playwright/test'
import type { Page } from '@playwright/test'

// Each live suite can run first; no file relies on another suite creating its account.
export async function signInAdmin(page: Page) {
  if (process.env.FERRIX_E2E_DISPOSABLE !== '1') throw new Error('Use a disposable browser-test database.')
  const setup = await (await page.request.get('/api/v1/auth/setup')).json()
  await page.goto(setup.setup_required ? '/signup' : '/login')
  if (setup.setup_required) await page.getByLabel('Name', { exact: true }).fill('Services UI test')
  await page.getByLabel('Email', { exact: true }).fill('services-ui@example.com')
  await page.getByLabel('Password', { exact: true }).fill('A long disposable browser test password!')
  await expect(page.getByText('Use two-factor authentication', { exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: setup.setup_required ? 'Create account' : 'Login', exact: true }).click()
  await expect(page).toHaveURL(/\/dashboard$/)
}
