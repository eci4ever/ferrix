import {test,expect} from '@playwright/test'
import {signInAdmin} from './helpers/session'
// Minimal ustar fixture keeps the browser test independent of system archive tools.
function archive(){const data=Buffer.from('FROM scratch\n');const h=Buffer.alloc(512);h.write('Dockerfile');h.write('0000644\0',100);h.write('0000000\0',108);h.write('0000000\0',116);h.write(data.length.toString(8).padStart(11,'0')+'\0',124);h.write('00000000000\0',136);h.fill(32,148,156);h.write('0',156);h.write('ustar\0',257);h.write('00',263);const sum=h.reduce((a,b)=>a+b,0);h.write(sum.toString(8).padStart(6,'0')+'\0 ',148);return Buffer.concat([h,data,Buffer.alloc(512-data.length),Buffer.alloc(1024)])}
test('Upload source preview and save a private immutable revision',async({page})=>{
 if(process.env.FERRIX_E2E_DISPOSABLE!=='1')throw new Error('Use disposable browser runner.')
 await signInAdmin(page);const headers={Origin:process.env.FERRIX_E2E_URL!}
 const project=await(await page.request.post('/api/v1/projects',{headers,data:{name:'Upload UI'}})).json();const env=(await(await page.request.get(`/api/v1/projects/${project.id}/environments`)).json()).environments[0]
 const app=await(await page.request.post(`/api/v1/environments/${env.id}/services`,{headers,data:{kind:'application',name:'Uploaded web',image:'nginx:alpine',container_port:80,environment:''}})).json()
 await page.goto(`/projects/${project.id}/services/${app.id}`);await page.getByRole('tab',{name:'Source',exact:true}).click();await page.getByRole('radio',{name:'Upload source archive',exact:true}).check()
 await page.getByLabel('Source archive',{exact:true}).setInputFiles({name:'app.tar',mimeType:'application/x-tar',buffer:archive()})
 await page.getByRole('button',{name:'Upload and preview',exact:true}).click();await expect(page.getByText('Preview ready. Review the build context before saving.',{exact:true})).toBeVisible()
 await page.getByText('Included files',{exact:true}).click();await expect(page.locator('pre').filter({hasText:'Dockerfile'})).toBeVisible()
 await page.getByRole('button',{name:'Use this revision',exact:true}).click();await expect(page.getByText('Uploaded revision saved. Deploy to build and activate it.',{exact:true})).toBeVisible()
 const source=await(await page.request.get(`/api/v1/services/${app.id}/source`)).json();expect(source.mode).toBe('upload');expect(source.sha256).toMatch(/^[a-f0-9]{64}$/)
 await page.setViewportSize({width:390,height:844});await expect.poll(()=>page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true)
})
