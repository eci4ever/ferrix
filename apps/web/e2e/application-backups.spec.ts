import {test,expect} from '@playwright/test'
import {randomUUID} from 'node:crypto'
import {signInAdmin} from './helpers/session'

test('Application Backups require offline consent and restore to a new stopped service',async({page})=>{
 if(process.env.FERRIX_E2E_DISPOSABLE!=='1')throw new Error('Use the disposable browser runner.')
 await signInAdmin(page);const headers={Origin:process.env.FERRIX_E2E_URL!}
 const project=await(await page.request.post('/api/v1/projects',{headers,data:{name:'Application backups UI'}})).json()
 const env=(await(await page.request.get(`/api/v1/projects/${project.id}/environments`)).json()).environments[0]
 const service=await(await page.request.post(`/api/v1/environments/${env.id}/services`,{headers,data:{kind:'application',name:'Backup worker UI',image:'nginx:alpine',worker_mode:true,storage:{target:'/opt/data',read_only:false}}})).json()
 const backupId=randomUUID(),hash='a'.repeat(64)
 const row={id:backupId,application_id:service.id,service_name:service.name,project_id:project.id,source_backup_id:null,operation:'backup',engine:'application',status:'succeeded',message:'Backup completed.',size_bytes:4096,sha256:hash,created_at:new Date().toISOString(),started_at:null,finished_at:new Date().toISOString()}
 await page.route(`**/api/v1/services/${service.id}/backups`,async route=>{
  if(route.request().method()==='GET')return route.fulfill({json:[row]})
  expect(route.request().postDataJSON()).toEqual({offline:true});return route.fulfill({status:202,json:{id:randomUUID(),status:'queued'}})
 })
 await page.goto(`/projects/${project.id}/services/${service.id}`)
 await page.getByRole('tab',{name:'Backups',exact:true}).click()
 await expect(page.getByRole('heading',{name:'Application volume backups',exact:true})).toBeVisible()
 await page.getByRole('button',{name:'Backup now',exact:true}).click()
 const consent=page.getByRole('dialog',{name:'Back up Application offline?',exact:true})
 await expect(consent).toContainText('This causes downtime')
 await consent.getByRole('button',{name:'Stop temporarily and back up',exact:true}).click()
 await expect(consent).toHaveCount(0)
 await expect(page.getByText('Offline backup request accepted.',{exact:true})).toBeVisible()
 await page.getByRole('checkbox',{name:'Enable scheduled backups',exact:true}).check()
 await page.getByRole('checkbox',{name:'I understand scheduled backups stop this service temporarily and restore its previous runtime state.',exact:true}).check()
 await page.getByLabel('Timezone',{exact:true}).fill('Asia/Kuala_Lumpur')
 await page.getByRole('button',{name:'Save backup settings',exact:true}).click()
 await expect(page.getByText('Backup schedule and storage saved.',{exact:true})).toBeVisible()
 const policy=await(await page.request.get(`/api/v1/services/${service.id}/backup-policy`)).json()
 expect(policy.enabled).toBe(true);expect(policy.application_offline).toBe(true);expect(policy.timezone).toBe('Asia/Kuala_Lumpur')
 await page.getByRole('button',{name:'Restore',exact:true}).click()
 const restore=page.getByRole('dialog',{name:'Restore to a new stopped Application',exact:true})
 await expect(restore).toContainText('target stays stopped')
 await expect(restore).toContainText('Failed targets remain blocked')
 await restore.getByRole('button',{name:'Cancel',exact:true}).click()
 await expect(restore).toHaveCount(0)
 await page.setViewportSize({width:390,height:844})
 await expect.poll(()=>page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true)
 await page.screenshot({path:'/private/tmp/ferrix-application-backups-mobile.png',fullPage:true})
})
