import { test, expect } from '@playwright/test'
import { signInAdmin } from './helpers/session'

test('Deployment notifications and an interactive service terminal respect session access', async ({ page, browser }) => {
 test.setTimeout(120_000)
 const base=process.env.FERRIX_E2E_URL!
 if(process.env.FERRIX_E2E_DISPOSABLE!=='1')throw new Error('Use the disposable browser runner.')
 await signInAdmin(page)
 const headers={Origin:base}
 const projectResponse=await page.request.post('/api/v1/projects',{headers,data:{name:'Terminal test'}})
 expect(projectResponse.status()).toBe(201)
 const project=await projectResponse.json()
 const environment=(await (await page.request.get(`/api/v1/projects/${project.id}/environments`)).json()).environments[0]
 const serviceResponse=await page.request.post(`/api/v1/environments/${environment.id}/services`,{headers,data:{kind:'application',name:'Terminal nginx',image:'nginx:alpine',container_port:80,environment:''}})
 expect(serviceResponse.status()).toBe(201)
 const service=await serviceResponse.json()
 expect((await page.request.post(`/api/v1/services/${service.id}/deploy`,{headers,data:{}})).status()).toBe(202)
 await expect.poll(async()=>{const history=await(await page.request.get(`/api/v1/services/${service.id}/deployments`)).json();const status=history[0]?.status;return ['succeeded','failed'].includes(status)?status:'pending'},{timeout:60_000}).not.toBe('pending')
 const finished=await(await page.request.get(`/api/v1/services/${service.id}/deployments`)).json()
 expect(finished[0].status,finished[0].logs).toBe('succeeded')
 await page.goto(`/projects/${project.id}/services/${service.id}`)
 await page.getByRole('button',{name:/^Notifications/}).click()
 const inbox=page.getByRole('dialog',{name:'Notifications',exact:true})
 await inbox.getByRole('button',{name:'Refresh',exact:true}).click()
 await expect(inbox).toContainText('Terminal nginx')
 await expect(inbox).toContainText('Deployment succeeded')
 await page.screenshot({path:'/private/tmp/ferrix-notifications.png',fullPage:true,animations:'disabled'})
 await inbox.getByRole('button',{name:'Mark all as read',exact:true}).click()
 await expect(inbox.getByRole('button',{name:'Mark all as read',exact:true})).toBeDisabled()
 await page.keyboard.press('Escape')
 await page.getByRole('button',{name:'Open terminal',exact:true}).click()
 let terminal=page.getByRole('dialog',{name:'Terminal · Terminal nginx',exact:true})
 await expect(terminal.getByRole('status')).toHaveText('Connected',{timeout:15_000})
 await terminal.getByLabel('Service terminal input',{exact:true}).focus()
 await page.keyboard.type("printf 'ferrix_%s\\n' terminal_ok\r")
 await expect(terminal.locator('.xterm-accessibility-tree')).toContainText('ferrix_terminal_ok')
 await page.screenshot({path:'/private/tmp/ferrix-service-terminal.png',fullPage:true,animations:'disabled'})
 await page.setViewportSize({width:390,height:844})
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true)
 await page.screenshot({path:'/private/tmp/ferrix-service-terminal-mobile.png',fullPage:true,animations:'disabled'})
 await terminal.getByRole('button',{name:'Close terminal',exact:true}).click()
 await expect(terminal).toHaveCount(0)
 // A foreign origin cannot turn a browser cookie into terminal access.
 const upgrade={Connection:'Upgrade',Upgrade:'websocket','Sec-WebSocket-Version':'13','Sec-WebSocket-Key':'dGhlIHNhbXBsZSBub25jZQ=='}
 const foreign=await page.request.get(`/api/v1/services/${service.id}/terminal`,{headers:{...upgrade,Origin:'https://untrusted.example'}})
 expect(foreign.status()).toBe(403)
 // An org member can read notifications, but cannot execute commands.
 const invite=await(await page.request.post('/api/v1/auth/invitations',{headers,data:{email:'terminal-member@example.com',role:'member'}})).json()
 const member=await browser.newContext({baseURL:base})
 try{
  expect((await member.request.post('/api/v1/auth/accept-invitation',{headers,data:{email:'terminal-member@example.com',name:'Terminal member',password:'Disposable member terminal password!',token:invite.token}})).status()).toBe(200)
  const memberInbox=await(await member.request.get('/api/v1/notifications')).json()
  expect(memberInbox.notifications.find((item:{service_id:string})=>item.service_id===service.id).read).toBe(false)
  expect((await member.request.get(`/api/v1/services/${service.id}/terminal`,{headers:{...upgrade,Origin:base}})).status()).toBe(403)
 }finally{await member.close()}
 // An open terminal closes promptly when this session is revoked.
 await page.setViewportSize({width:1440,height:1000})
 await page.getByRole('button',{name:'Open terminal',exact:true}).click()
 terminal=page.getByRole('dialog',{name:'Terminal · Terminal nginx',exact:true})
 await expect(terminal.getByRole('status')).toHaveText('Connected',{timeout:15_000})
 expect((await page.request.post('/api/v1/auth/sign-out',{headers,data:{}})).status()).toBe(204)
 await expect(terminal.getByRole('status')).toHaveText('Disconnected',{timeout:20_000})
 await expect(terminal).toContainText('session, permission or service changed')
})
