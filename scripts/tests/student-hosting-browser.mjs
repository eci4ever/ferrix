import {chromium, expect} from '../../apps/web/node_modules/@playwright/test/index.mjs'
import {mkdtemp,writeFile,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
const origin=process.env.STUDENT_TEST_ORIGIN
const browser=await chromium.launch({headless:true})
const folder=await mkdtemp(join(tmpdir(),'ferrix-student-upload-'))
async function login(page,email,password){
 await page.goto(origin+'/login')
 await page.getByLabel('Email',{exact:true}).fill(email)
 await page.getByLabel('Password',{exact:true}).fill(password)
 await Promise.all([page.waitForURL(url=>url.pathname!=='/login'),page.getByRole('button',{name:'Login',exact:true}).click()])
}
try{
 const page=await browser.newPage({viewport:{width:1440,height:1050}})
 await login(page,'alice@example.test',process.env.STUDENT_TEST_PASSWORD)
 await expect(page).toHaveURL(origin+'/student-hosting')
 await expect(page.getByRole('heading',{name:'Database connection',exact:true})).toBeVisible()
 await expect(page.getByRole('link',{name:'Projects',exact:true})).toHaveCount(0)
 await page.getByRole('button',{name:'Show password',exact:true}).click()
 await expect(page.getByLabel('Database password')).toHaveAttribute('type','text')
 await page.getByRole('button',{name:'Hide password',exact:true}).click()
 await writeFile(join(folder,'index.php'),'<?php echo "<h1>Student upload works</h1>"; echo getenv("DB_NAME") ? "Database configured" : "Missing database";')
 await page.getByLabel('Project folder').setInputFiles(folder)
 await page.getByRole('button',{name:'Upload project',exact:true}).click()
 await expect(page.getByText('Files uploaded. Open your website to see the changes.')).toBeVisible()
 const href=await page.getByRole('link',{name:'Open website',exact:true}).getAttribute('href')
 const website=await browser.newPage();await website.goto(href)
 await expect(website.getByRole('heading',{name:'Student upload works',exact:true})).toBeVisible()
 await page.screenshot({path:'/tmp/ferrix-student-hosting-desktop.png',fullPage:true})
 await page.setViewportSize({width:390,height:844})
 await page.screenshot({path:'/tmp/ferrix-student-hosting-mobile.png',fullPage:true})
 if(!await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth))throw new Error('Student page overflows on mobile')
 const admin=await browser.newPage()
 await login(admin,'teacher@example.test',process.env.STUDENT_TEST_TEACHER_PASSWORD)
 await admin.goto(origin+'/student-hosting')
 await expect(admin.getByRole('heading',{name:'Shared MariaDB',exact:true})).toBeVisible()
 await expect(admin.getByRole('table',{name:'Students',exact:true}).getByText('alice@example.test')).toBeVisible()
 await admin.screenshot({path:'/tmp/ferrix-student-hosting-admin.png',fullPage:true})
 await admin.getByRole('button',{name:'Close registration',exact:true}).click()
 const registration=await browser.newPage();await registration.goto(origin+'/student-register')
 await expect(registration.getByText('Student registration is currently closed. Contact your teacher.')).toBeVisible()
 console.log('Browser passed: student login/redirect, private dashboard, upload and working PHP, mobile layout, teacher dashboard and registration closure.')
}finally{await browser.close();await rm(folder,{recursive:true,force:true})}
