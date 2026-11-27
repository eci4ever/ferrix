import { test, expect } from '@playwright/test'

for (const setup of [false, true]) test(`Signup hides social actions and preserves ${setup ? 'first-admin setup' : 'required invitation token'}`, async ({ page }) => {
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/v1/auth/session') return route.fulfill({ status: 401, json: { error: 'Not signed in' } })
    return route.fulfill({ json: path === '/api/v1/auth/setup' ? { setup_required: setup } : {} })
  })
  await page.goto('/signup')
  await expect(page.getByLabel('Name', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: /^Continue with / })).toHaveCount(0)
  await expect(page.getByText('Or continue with', { exact: true })).toHaveCount(0)
  if (setup) {
    await expect(page.getByRole('button', { name: 'Create account', exact: true })).toBeVisible()
    await expect(page.getByLabel('Invitation token', { exact: true })).toHaveCount(0)
  } else {
    await expect(page.getByLabel('Invitation token', { exact: true })).toHaveAttribute('required', '')
    await expect(page.getByRole('button', { name: 'Accept invitation', exact: true })).toBeVisible()
    await page.getByRole('link', { name: 'Sign in', exact: true }).click()
    await expect(page.getByRole('button', { name: 'Continue with Google', exact: true })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Continue with GitHub', exact: true })).toBeVisible()
  }
})

test('Recipient declines with email and invitation token without entering account credentials', async ({ page }) => {
  const proofs: unknown[] = []
  await page.route('**/api/v1/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/auth/session')) return route.fulfill({ status: 401, json: { error: 'Not signed in' } })
    if (path.endsWith('/auth/decline-invitation')) { proofs.push(route.request().postDataJSON()); return route.fulfill({ status: 204 }) }
    return route.fulfill({ json: path.endsWith('/auth/setup') ? { setup_required: false } : {} })
  })
  await page.goto('/signup')
  await page.getByRole('button', { name: 'Decline invitation', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('Enter your email and invitation token')
  expect(proofs).toEqual([])
  await page.getByLabel('Email', { exact: true }).fill('recipient@example.com')
  await page.getByLabel('Invitation token', { exact: true }).fill('private-token')
  await page.getByRole('button', { name: 'Decline invitation', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('Invitation declined')
  expect(proofs).toEqual([{ email: 'recipient@example.com', token: 'private-token' }])
  await expect(page.getByLabel('Password', { exact: true })).toHaveValue('')
})
