import { test, expect } from '@playwright/test'

for (const kind of ['application', 'database']) test(`Service notes persist drafts and enforce permissions for ${kind}`, async ({ page }) => {
  let member = false
  let conflict = false
  let saved = { notes: '', revision: 0 }
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname.replace('/api/v1', '')
    if (path === '/services/demo/notes') {
      if (route.request().method() === 'POST') {
        expect(route.request().headers()['content-type']).toContain('application/json')
        if (conflict) return route.fulfill({ status:409, json:{ error:'Notes were changed by someone else. Copy your draft before loading the latest saved notes.' } })
        const data = route.request().postDataJSON()
        expect(data.revision).toBe(saved.revision)
        saved = { notes:data.notes, revision:saved.revision + 1 }
      }
      return route.fulfill({ json:saved })
    }
    const responses: Record<string, unknown> = {
      '/auth/session':{ name:'Admin', system_role:'admin', role:member ? 'member' : 'owner', organization_id:'org',organization_name:'Team' },
      '/auth/setup':{ setup_required:false }, '/auth/organizations':[], '/status':{ database:true,docker:true,version:'test' },
      '/services/demo':{ id:'demo',environment_id:'dev',project_id:'project',kind,name:'Demo',image:'nginx:alpine',created_at:new Date().toISOString(),database_engine:'postgres' },
      '/services/demo/runtime':{ state:'not_deployed' }, '/services/demo/deployments':[],
      '/projects/project/environments':{ project_name:'Test',environments:[{id:'dev',name:'development'}] },
    }
    return route.fulfill({json:responses[path] ?? {}})
  })
  await page.goto('/projects/project/services/demo')
  await page.getByRole('tab', { name:'Notes',exact:true }).click()
  const input = page.getByRole('textbox', { name:'Notes',exact:true })
  await input.fill('Setup steps\n<script>alert("text only")</script>')
  await page.getByRole('tab',{name:'Overview',exact:true}).click()
  await page.getByRole('tab',{name:'Notes',exact:true}).click()
  await expect(input).toHaveValue('Setup steps\n<script>alert("text only")</script>')
  await page.getByRole('button',{name:'Save notes',exact:true}).click()
  await expect(page.getByText('Notes saved. No deployment needed.')).toBeVisible()
  await page.reload()
  await page.getByRole('tab',{name:'Notes',exact:true}).click()
  await expect(input).toHaveValue(saved.notes)
  conflict = true
  await input.fill('Unsaved local draft')
  await page.getByRole('button',{name:'Save notes',exact:true}).click()
  await expect(page.getByRole('alert')).toContainText('changed by someone else')
  await expect(input).toHaveValue('Unsaved local draft')
  await page.getByRole('button',{name:'Discard draft and load saved notes'}).click()
  await expect(input).toHaveValue(saved.notes)
  member = true
  await page.reload()
  await page.getByRole('tab',{name:'Notes',exact:true}).click()
  await expect(input).toHaveAttribute('readonly','')
  await expect(page.getByRole('button',{name:'Save notes',exact:true})).toHaveCount(0)
  await page.setViewportSize({width:320,height:800})
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})
