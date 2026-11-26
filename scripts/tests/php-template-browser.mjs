// Real isolated API/browser, with sessions supplied only over stdin by the runner.
import { chromium } from '../../apps/web/node_modules/playwright/index.mjs'
import assert from 'node:assert/strict'
let input = ''
for await (const chunk of process.stdin) input += chunk
const fixture = JSON.parse(input)
const browser = await chromium.launch({ headless: true })
try {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } })
  await context.addCookies(fixture.managerCookies)
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto(`${fixture.origin}/projects/${fixture.project}`)
  await page.getByRole('button', { name: 'Add student PHP application', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Application name', { exact: true }).fill('PHP browser fixture')
  await dialog.getByLabel('Student', { exact: true }).selectOption(fixture.student)
  await dialog.getByLabel('Shared MariaDB service', { exact: true }).selectOption(fixture.server)
  await dialog.getByText('Preview generated files and build conventions', { exact: true }).click()
  await dialog.getByRole('link', { name: 'Download starter source', exact: true }).waitFor()
  const creating = page.waitForResponse(res => res.url().endsWith(`/environments/${fixture.environment}/php-applications`) && res.request().method() === 'POST')
  await dialog.getByRole('button', { name: 'Create assigned application', exact: true }).click()
  const created = await creating
  assert.equal(created.status(), 200)
  const application = await created.json()
  const provisioned = page.waitForResponse(res => res.url().endsWith(`/${application.application_id}/php-template/provision`) && res.request().method() === 'POST')
  await dialog.getByRole('button', { name: 'Provision database and connection settings', exact: true }).click()
  assert.equal((await provisioned).status(), 200)
  const deployed = page.waitForResponse(res => res.url().endsWith(`/${application.application_id}/deploy`) && res.request().method() === 'POST')
  await dialog.getByRole('button', { name: 'Deploy PHP application', exact: true }).click()
  assert.equal((await deployed).status(), 202)
  await dialog.getByRole('button', { name: 'Close', exact: true }).click()
  await page.getByRole('link', { name: 'PHP browser fixture', exact: true }).waitFor()
  await context.close()

  const student = await browser.newContext()
  await student.addCookies(fixture.studentCookies)
  const own = await student.newPage()
  own.on('pageerror', error => errors.push(error.message))
  await own.goto(`${fixture.origin}/projects/${fixture.project}/services/${fixture.application}`)
  await own.getByRole('tab', { name: 'Database', exact: true }).click()
  await own.getByRole('heading', { name: 'PHP classroom template', exact: true }).waitFor()
  await own.getByText('Public URL', { exact: true }).waitFor()
  await own.getByText('LAN URL', { exact: true }).waitFor()
  assert.equal(await own.getByRole('button', { name: 'Retry database provisioning', exact: true }).count(), 0)
  await own.getByRole('tab', { name: 'Source', exact: true }).click()
  await own.getByLabel('Build method', { exact: true }).waitFor()
  assert.equal(await own.getByLabel('Build method', { exact: true }).inputValue(), 'php-v1')
  await student.close()

  const lab = await browser.newContext()
  const external = []
  await lab.route('**/*', route => {
    if (new URL(route.request().url()).hostname === '127.0.0.1') return route.continue()
    external.push(route.request().url()); return route.abort()
  })
  const offline = await lab.newPage()
  await offline.goto(fixture.starterUrl)
  await offline.getByRole('heading', { name: 'Your application is running.', exact: true }).waitFor()
  await offline.getByText('Database connected', { exact: true }).waitFor()
  assert((await offline.locator('body').evaluate(node => getComputedStyle(node).fontFamily)).includes('system-ui'))
  assert.deepEqual(external, [])
  assert.deepEqual(errors, [])
  console.log('PASS: real browser creates/provisions/deploys PHP via wizard; student build controls and separate public/LAN status render; starter works with external browser requests blocked.')
} finally {
  await browser.close()
}
