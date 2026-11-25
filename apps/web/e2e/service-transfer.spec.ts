import { test, expect } from '@playwright/test'

for (const operation of ['move','clone']) test(`${operation} service uses a destination dialog and retains input on error`, async ({page}) => {
  let failed = true
  let environment = 'dev'
  let visited = false
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname.replace('/api/v1','')
    if (path === `/services/web/${operation}`) {
      expect(route.request().headers()['content-type']).toContain('application/json')
      expect(route.request().postDataJSON()).toEqual({environment_id:'prod',...(operation === 'clone' ? {name:'Production web'} : {})})
      if (failed) { failed = false; return route.fulfill({status:409,json:{error:'Destination name conflict. Choose another name or environment.'}}) }
      visited = true; environment = 'prod'
      return route.fulfill({status:operation === 'clone' ? 201 : 200,json:{id:operation === 'clone' ? 'copy' : 'web'}})
    }
    const service = {id:path.includes('/copy') ? 'copy':'web',name:'Web',kind:'application',project_id:'project',environment_id:environment,image:'nginx:alpine',created_at:new Date().toISOString()}
    const responses:Record<string,unknown> = {
      '/auth/session':{name:'Admin',system_role:'admin',role:'owner',organization_id:'org',organization_name:'Test'},'/auth/setup':{setup_required:false},'/auth/organizations':[],
      '/status':{database:true,docker:true,version:'test'},
      '/projects/project/environments':{project_name:'Example',environments:[{id:'dev',name:'development'},{id:'prod',name:'production'}]},
      '/services/web':service,'/services/copy':service,
      '/services/web/runtime':{state:'not_deployed'},'/services/copy/runtime':{state:'not_deployed'},
      '/services/web/deployments':[],'/services/copy/deployments':[],
      '/services/web/notes':{notes:'',revision:0},'/services/copy/notes':{notes:'',revision:0},
    }
    return route.fulfill({json:responses[path] ?? {}})
  })
  await page.setViewportSize({width:320,height:800})
  await page.goto('/projects/project/services/web')
  const title = operation === 'move' ? 'Move service' : 'Clone to environment'
  const trigger = page.getByRole('button',{name:title,exact:true})
  await trigger.click()
  const dialog = page.getByRole('dialog',{name:title,exact:true})
  await expect(dialog.getByRole('button',{name:operation === 'move' ? title:'Clone service',exact:true})).toBeDisabled()
  await expect(dialog.getByLabel('Destination environment').locator('option')).toHaveCount(2)
  await dialog.getByLabel('Destination environment').selectOption('prod')
  if (operation === 'clone') await dialog.getByLabel('Service name',{exact:true}).fill('Production web')
  const submit = dialog.getByRole('button',{name:operation === 'move' ? title:'Clone service',exact:true})
  await submit.scrollIntoViewIfNeeded()
  await submit.click()
  await expect(dialog.getByRole('alert').filter({hasText:'Unable to continue'})).toContainText('Destination name conflict')
  await expect(dialog.getByLabel('Destination environment')).toHaveValue('prod')
  await submit.click()
  await expect(page).toHaveURL(new RegExp(`/services/${operation === 'clone' ? 'copy':'web'}$`))
  await expect(dialog).toHaveCount(0)
  expect(visited).toBe(true)
})
