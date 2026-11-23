import { expect, test } from '@playwright/test'

test('logs auto-refresh without overlaps and pause on demand, hidden tabs, close or access loss', async ({ page }) => {
  await page.clock.install()
  let requests = 0
  let message = 'first snapshot'
  let hold = false
  let release: (() => void) | undefined
  let denied = false
  let unavailable = false
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/logs')) {
      requests++
      if (hold) await new Promise<void>(resolve => { release = resolve })
      await route.fulfill(denied ? {status:403,json:{error:'Administrator access required.'}} : unavailable ? {status:503,json:{error:'Docker unavailable.'}} : {json:{logs:message,truncated:false,checked_at:new Date().toISOString()}})
      return
    }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': {name:'Admin',system_role:'admin',organization_id:'org',organization_name:'Test',role:'owner'},
      '/api/v1/auth/organizations': [], '/api/v1/auth/setup': {setup_required:false},
      '/api/v1/status': {database:true,docker:true,version:'0.2.0'},
      '/api/v1/server-time': {unix_ms:Date.now(),timezone:'UTC',utc_offset:'+00:00'},
      '/api/v1/containers': [],
      '/api/v1/docker/swarm': {state:'active',manager:true,node_list:[],service_list:[{id:'aaaaaaaaaaaaaaaaaaaaaaaaa',name:'demo',mode:'Replicated',replicas:2,version:12,running_tasks:2,desired_tasks:2}]},
    }
    return route.fulfill({json:responses[path] ?? {}})
  })
  await page.goto('/containers')
  await page.getByRole('tab',{name:'Swarm',exact:true}).click()
  await page.getByRole('tab',{name:'Services',exact:true}).click()
  await page.getByRole('button',{name:'View logs for demo',exact:true}).click()
  const logs = page.getByLabel('Swarm service logs',{exact:true})
  await expect(logs).toHaveText('first snapshot')
  message = 'automatic snapshot'
  await page.clock.runFor(3000)
  await expect(logs).toHaveText(message)
  await page.getByRole('button',{name:'Pause auto-refresh',exact:true}).click()
  const pausedCount = requests
  message = 'paused snapshot'
  await page.clock.runFor(9000)
  expect(requests).toBe(pausedCount)
  await expect(logs).toHaveText('automatic snapshot')
  await page.getByRole('button',{name:'Refresh logs',exact:true}).click()
  await expect(logs).toHaveText(message)
  expect(requests).toBe(pausedCount + 1)
  await page.clock.runFor(6000)
  expect(requests).toBe(pausedCount + 1)
  await page.getByRole('button',{name:'Resume auto-refresh',exact:true}).click()
  await expect.poll(()=>requests).toBe(pausedCount + 2)
  await expect(page.getByRole('button',{name:'Refresh logs',exact:true})).toBeEnabled()
  hold = true
  await page.clock.runFor(3000)
  await expect.poll(()=>release !== undefined).toBe(true)
  const heldCount = requests
  await page.clock.runFor(6000)
  expect(requests).toBe(heldCount)
  hold = false; release?.()
  await expect(page.getByRole('button',{name:'Refresh logs',exact:true})).toBeEnabled()
  await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'))})
  const hiddenCount = requests
  await page.clock.runFor(9000)
  expect(requests).toBe(hiddenCount)
  message = 'visible again'
  await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,value:false});document.dispatchEvent(new Event('visibilitychange'))})
  await expect(logs).toHaveText(message)
  unavailable = true
  await page.clock.runFor(3000)
  await expect(page.getByText(/Showing the last successful snapshot/)).toBeVisible()
  await expect(logs).toHaveText(message)
  unavailable = false
  message = 'recovered snapshot'
  await page.clock.runFor(3000)
  await expect(logs).toHaveText(message)
  denied = true
  await page.clock.runFor(3000)
  await expect(page.getByText('Administrator access required.',{exact:true})).toBeVisible()
  await expect(logs).toHaveCount(0)
  await expect(page.getByRole('button',{name:'Resume auto-refresh',exact:true})).toBeVisible()
  const deniedCount = requests
  await page.clock.runFor(9000)
  expect(requests).toBe(deniedCount)
  denied = false
  message = 'access restored'
  await page.getByRole('button',{name:'Resume auto-refresh',exact:true}).click()
  await expect(logs).toHaveText(message)
  const closedCount = requests
  await page.getByRole('button',{name:'Close logs',exact:true}).click()
  await page.clock.runFor(9000)
  expect(requests).toBe(closedCount)
})
