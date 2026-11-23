import { expect, test } from '@playwright/test'

test('service actions confirm mutations and display bounded log snapshots with recovery', async ({ page }) => {
  let replicas = 2
  let updates = 0
  let conflict = false
  let restarts = 0
  let logMode = 'ok'
  let logRequests = 0
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/logs')) {
      logRequests++
      if (logMode === 'error') return route.fulfill({status:503,json:{error:'Docker logging driver unavailable.'}})
      return route.fulfill({json:{logs:logMode === 'empty' ? '' : '\u001b[32m2026-11-23T00:00:00Z nginx GET /demo <img src=x onerror=alert(1)>\u001b[0m',truncated:logMode === 'truncated',checked_at:'2026-11-23T00:00:00Z'}})
    }
    if (path.endsWith('/restart')) {
      restarts++
      expect(route.request().postDataJSON()).toEqual({version:12})
      if (conflict) return route.fulfill({status:409,json:{error:'Service changed. Refresh swarm before updating it again.'}})
      return route.fulfill({status:204})
    }
    if (path.endsWith('/scale')) {
      updates++
      expect(route.request().postDataJSON()).toEqual({replicas:3,version:12})
      if (conflict) return route.fulfill({status:409,json:{error:'Service changed. Refresh swarm before scaling again.'}})
      replicas = 3
      return route.fulfill({status:204})
    }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': { name:'Admin',system_role:'admin',organization_id:'org',organization_name:'Test',role:'owner' },
      '/api/v1/auth/organizations': [], '/api/v1/auth/setup': {setup_required:false},
      '/api/v1/status': {database:true,docker:true,version:'0.2.0'},
      '/api/v1/server-time': {unix_ms:Date.now(),timezone:'UTC',utc_offset:'+00:00'},
      '/api/v1/containers': [],
      '/api/v1/docker/swarm': {state:'active',manager:true,node_list:[],service_list:[
        {id:'aaaaaaaaaaaaaaaaaaaaaaaaa',name:'demo',mode:'Replicated',replicas,version:12,running_tasks:replicas,desired_tasks:replicas},
        {id:'global',name:'global-agent',mode:'Global'}, {id:'job',name:'backup',mode:'ReplicatedJob'},
      ]},
    }
    return route.fulfill({json:responses[path] ?? {}})
  })
  await page.goto('/containers')
  await page.getByRole('tab',{name:'Swarm',exact:true}).click()
  await page.getByRole('tab',{name:'Services',exact:true}).click()
  await expect(page.getByRole('button',{name:'Scale global-agent',exact:true})).toHaveCount(0)
  await expect(page.getByRole('button',{name:'Scale backup',exact:true})).toHaveCount(0)
  await page.getByRole('button',{name:'Scale demo',exact:true}).click()
  await expect(page.getByRole('button',{name:'Scale service',exact:true})).toBeDisabled()
  await page.getByLabel('Replicas',{exact:true}).fill('0')
  await expect(page.getByText('This stops all service tasks',{exact:true})).toBeVisible()
  await page.getByLabel('Replicas',{exact:true}).fill('101')
  await expect(page.getByRole('button',{name:'Scale service',exact:true})).toBeDisabled()
  await page.getByLabel('Replicas',{exact:true}).fill('1.5')
  await expect(page.getByRole('button',{name:'Scale service',exact:true})).toBeDisabled()
  await page.getByRole('button',{name:'Cancel',exact:true}).click()
  expect(updates).toBe(0)
  await page.getByRole('button',{name:'Scale demo',exact:true}).click()
  await page.getByLabel('Replicas',{exact:true}).fill('3')
  conflict = true
  await page.getByRole('button',{name:'Scale service',exact:true}).click()
  await expect(page.getByText('Service changed. Refresh swarm before scaling again.',{exact:true})).toBeVisible()
  conflict = false
  await page.getByRole('button',{name:'Scale service',exact:true}).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByRole('cell',{name:'3 / 3',exact:true})).toBeVisible()
  await expect(page.getByText(/Scale requested for demo: 3 replicas/)).toBeVisible()
  await expect(page.getByRole('tab',{name:'Services',exact:true})).toHaveAttribute('aria-selected','true')
  expect(updates).toBe(2)
  await expect(page.getByRole('button',{name:'Restart backup',exact:true})).toHaveCount(0)
  await expect(page.getByRole('button',{name:'Restart global-agent',exact:true})).toBeDisabled()
  await page.getByRole('button',{name:'Restart demo',exact:true}).click()
  await expect(page.getByRole('dialog')).toContainText('Connections may be interrupted')
  await page.getByRole('button',{name:'Cancel',exact:true}).click()
  expect(restarts).toBe(0)
  await page.getByRole('button',{name:'Restart demo',exact:true}).click()
  conflict = true
  await page.getByRole('button',{name:'Restart service',exact:true}).click()
  await expect(page.getByText('Service could not be restarted',{exact:true})).toBeVisible()
  conflict = false
  await page.getByRole('button',{name:'Restart service',exact:true}).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByText(/Restart requested for demo/)).toBeVisible()
  await expect(page.getByRole('cell',{name:'3 / 3',exact:true})).toBeVisible()
  expect(restarts).toBe(2)
  expect(logRequests).toBe(0)
  await page.getByRole('button',{name:'View logs for demo',exact:true}).click()
  await expect(page.getByLabel('Swarm service logs',{exact:true})).toContainText('nginx GET /demo <img src=x onerror=alert(1)>')
  await expect(page.locator('pre img')).toHaveCount(0)
  await expect(page.getByLabel('Swarm service logs',{exact:true})).not.toContainText('\u001b')
  await page.setViewportSize({width:390,height:844})
  expect(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  logMode = 'empty'
  await page.getByRole('button',{name:'Refresh logs',exact:true}).click()
  await expect(page.getByText('No logs reported by Docker for this service.',{exact:true})).toBeVisible()
  logMode = 'error'
  await page.getByRole('button',{name:'Refresh logs',exact:true}).click()
  await expect(page.getByText('Service logs unavailable',{exact:true})).toBeVisible()
  logMode = 'truncated'
  await page.getByRole('button',{name:'Refresh logs',exact:true}).click()
  await expect(page.getByText('Log snapshot truncated',{exact:true})).toBeVisible()
  await page.getByRole('button',{name:'Close logs',exact:true}).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByRole('tab',{name:'Services',exact:true})).toHaveAttribute('aria-selected','true')
})
