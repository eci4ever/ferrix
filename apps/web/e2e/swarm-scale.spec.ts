import { expect, test } from '@playwright/test'

test('scale and restart require confirmation, handle conflicts and retain replica counts', async ({ page }) => {
  let replicas = 2
  let updates = 0
  let conflict = false
  let restarts = 0
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname
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
})
