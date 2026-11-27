import { test, expect, type Page } from '@playwright/test'

async function workspace(page: Page, member = false) {
  let permission = 'developer', restricted = false
  const service = { id: 'a', project_id: 'class', environment_id: 'production', kind: 'application', name: 'Application A', image: 'nginx:alpine', deployed: false, created_at: new Date().toISOString() }
  const config = { name: 'Application A', image: 'nginx:alpine', container_port: 80, environment: 'MODE=test', memory_mib: 256, cpus: 0.5, worker_mode: false, command: [], storage: null }
  await page.route('**/api/v1/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON()
      if (path.endsWith('/assignments')) { expect(body).toEqual({user_id:'student',permission:'developer'}); permission = body.permission }
      if (path.endsWith('/organizations/access')) restricted = body.restricted
      if (path.endsWith('/configuration')) { expect(body.memory_mib).toBe(256); expect(body.cpus).toBe(0.5); return route.fulfill({json:{...service,name:body.name}}) }
      return route.fulfill({status:204})
    }
    const data: Record<string, unknown> = {
      '/api/v1/auth/session': {user_id:'user',name:'Lecturer',email:'user@example.com',system_role:'user',role:member?'member':'admin',organization_id:'org',organization_name:'Classroom'},
      '/api/v1/auth/setup': {setup_required:false}, '/api/v1/auth/organizations':[{id:'org',name:'Classroom',role:member?'member':'admin'}],
      '/api/v1/auth/members': [{id:'student',name:'Student A',email:'student@example.com',role:'member'}], '/api/v1/auth/invitations':[],
      '/api/v1/auth/organizations/current':{id:'org',name:'Classroom',logo_url:null,is_default:false}, '/api/v1/auth/organizations/access':{restricted},
      '/api/v1/status': {database:true,docker:true,version:'test'},
      '/api/v1/projects/class/environments':{project_name:'Class',environments:[{id:'production',name:'production'}]},
      '/api/v1/environments/production/services':[service], '/api/v1/environments/production/service-status':[],
      '/api/v1/projects/class/assignments':{assignments:[]}, '/api/v1/services/a/assignments':{assignments:[{user_id:'student',permission}]},
      '/api/v1/services/a':service, '/api/v1/services/a/access':{permission,can_develop:permission==='developer',can_admin:false},
      '/api/v1/services/a/configuration':config, '/api/v1/services/a/runtime':{state:'not_deployed'}, '/api/v1/services/a/deployments':[], '/api/v1/services/a/notes':{notes:'',revision:0},
    }
    return route.fulfill({json:data[path]??{}})
  })
}

test('Organization admins configure classroom restriction and service assignments', async ({ page }) => {
  await workspace(page)
  await page.goto('/organization')
  await page.getByRole('tab',{name:'Access',exact:true}).click()
  await page.getByRole('button',{name:'Configure member visibility'}).click()
  await page.getByRole('checkbox',{name:'Restrict members to assigned projects and services'}).check()
  await page.getByRole('button',{name:'Save member visibility'}).click()
  await expect(page.getByText('Member visibility saved.')).toBeVisible()
  await page.goto('/projects/class')
  await page.getByRole('button',{name:'Manage access',exact:true}).click()
  const dialog=page.getByRole('dialog',{name:'Manage resource access'})
  await dialog.getByRole('combobox', {name:'Organization member'}).fill('no-such-member')
  await dialog.getByRole('combobox', {name:'Organization member'}).press('ArrowDown')
  await expect(page.getByText('No matching members.')).toBeVisible()
  await dialog.getByRole('combobox', {name:'Organization member'}).press('Escape')
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('combobox', {name:'Organization member'})).toHaveAttribute('aria-expanded', 'false')
  await expect(dialog.getByRole('button',{name:'Save access'})).toBeDisabled()
  await dialog.getByRole('combobox', {name:'Organization member'}).fill('student@example.com')
  await page.getByRole('option', {name:/Student A/}).click()
  await expect(dialog.getByRole('button',{name:'Save access'})).toBeEnabled()
  await dialog.getByRole('button',{name:'Clear selection'}).click()
  await expect(dialog.getByRole('button',{name:'Save access'})).toBeDisabled()
  await dialog.getByRole('combobox', {name:'Organization member'}).fill('student@example.com')
  await dialog.getByRole('combobox', {name:'Organization member'}).press('ArrowDown')
  await dialog.getByRole('combobox', {name:'Organization member'}).press('Enter')
  await dialog.getByLabel('Resource permission').selectOption('developer')
  await dialog.getByRole('button',{name:'Save access'}).click()
  await expect(page.getByText('Resource access saved.')).toBeVisible()
  await dialog.locator('form').getByRole('button',{name:'Close',exact:true}).click()
  await page.goto('/projects/class/services/a')
  await expect(page.getByRole('button',{name:'Manage access',exact:true})).toBeVisible()
})

test('Assigned developers edit approved settings without manager actions or resource ceiling edits', async ({ page }) => {
  await workspace(page,true)
  await page.goto('/projects/class/services/a')
  await expect(page.getByText('Access: developer',{exact:true})).toBeVisible()
  await expect(page.getByRole('button',{name:'Deploy',exact:true})).toBeVisible()
  await expect(page.getByRole('button',{name:'Manage access',exact:true})).toHaveCount(0)
  await expect(page.getByRole('button',{name:'Move service',exact:true})).toHaveCount(0)
  await page.getByRole('tab',{name:'Configuration',exact:true}).click()
  await page.getByRole('button',{name:'Load configuration',exact:true}).click()
  await expect(page.getByLabel('Memory limit (MiB)',{exact:true})).toBeDisabled()
  await expect(page.getByLabel('CPU limit (cores)',{exact:true})).toBeDisabled()
  await page.getByLabel('Application name',{exact:true}).fill('Updated A')
  await page.getByRole('button',{name:'Save configuration',exact:true}).click()
  await expect(page.getByText('Configuration saved. Deploy to apply it.')).toBeVisible()
  await page.getByRole('tab',{name:'Backups',exact:true}).click()
  await expect(page.getByText('Backups and restores are managed by your organization managers.')).toBeVisible()
})
