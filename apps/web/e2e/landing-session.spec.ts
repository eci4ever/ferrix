import { expect, test } from '@playwright/test'

for (const signedIn of [false, true]) {
  test(`landing actions reflect ${signedIn ? 'an active session' : 'a guest session'}`, async ({ page }) => {
    await page.route('**/api/v1/**', route => {
      const path = new URL(route.request().url()).pathname
      if (path === '/api/v1/auth/session' && !signedIn) return route.fulfill({status:401,json:{error:'Session expired.'}})
      const responses: Record<string, unknown> = {
        '/api/v1/auth/session': {name:'Admin',system_role:'admin',organization_id:'org',organization_name:'Test',role:'owner'},
        '/api/v1/auth/setup': {setup_required:false}, '/api/v1/auth/organizations': [],
        '/api/v1/status': {database:true,docker:true,version:'0.2.4'},
        '/api/v1/projects': [],
        '/api/v1/home-summary': {projects:0,services:0,deployments_24h:0,projects_new_7d:0,attention:[],needs_attention:0},
      }
      return route.fulfill({json:responses[path] ?? {}})
    })
    await page.setViewportSize({width:390,height:844})
    await page.goto('/')
    if (signedIn) {
      await expect(page.getByRole('link',{name:'Dashboard',exact:true})).toHaveCount(2)
      await expect(page.getByRole('link',{name:'Sign in',exact:true})).toHaveCount(0)
      await expect(page.getByRole('link',{name:'Get started',exact:true})).toHaveCount(0)
      await page.getByRole('link',{name:'Dashboard',exact:true}).first().click()
      await expect(page).toHaveURL(/\/dashboard$/)
    } else {
      await expect(page.getByRole('link',{name:'Dashboard',exact:true})).toHaveCount(0)
      await expect(page.getByRole('link',{name:'Sign in',exact:true})).toHaveAttribute('href','/login')
      await expect(page.getByRole('link',{name:'Get started',exact:true})).toHaveCount(2)
    }
  })
}
