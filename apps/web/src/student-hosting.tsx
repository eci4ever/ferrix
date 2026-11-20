import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { ExternalLink, Eye, EyeOff, Layers3 } from 'lucide-react'
import { request } from '@/lib/api'
import type { Identity } from '@/auth'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Input } from '@/components/ui/input'
import { InputGroup, InputGroupInput, InputGroupAddon, InputGroupButton } from '@/components/ui/input-group'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Badge } from '@/components/ui/badge'
import { HostingTools } from '@/hosting-tools'
import { DataTable } from '@/components/data-table'

type Hosting = {user_id:string;name:string;email:string;status:string;database_name:string;database_user:string;database_host?:string;database_port?:number;database_password?:string;automatic_url:string|null;error:string|null}
type Settings = {enabled:boolean;registration_open:boolean;server_state:string;error:string|null}
function errorText(e:unknown) {return e instanceof Error?e.message:'Request failed. Please retry.'}
function Message({message}:{message:string}){return message?<Alert><AlertTitle>Hosting</AlertTitle><AlertDescription>{message}</AlertDescription></Alert>:null}
export function StudentRegistration() {
  const [open, setOpen] = useState<boolean | null>(null)
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState(false)
  const [submitted, setSubmitted] = useState(false)
  const [showPassword, setShowPassword] = useState(false)

  async function checkRegistration() {
    setMessage('')
    try {
      const result = await request<{ registration_open: boolean }>('/auth/student-registration')
      setOpen(result.registration_open)
    } catch (error) { setMessage(errorText(error)) }
  }
  useEffect(() => { void checkRegistration() }, [])

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    setBusy(true); setMessage('')
    try {
      await request('/auth/student-register', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(Object.fromEntries(data)),
      })
      setSubmitted(true)
    } catch (error) { setMessage(errorText(error)) }
    finally { setBusy(false) }
  }

  const title = submitted ? 'Registration received' : open === false ? 'Registration closed' : 'Create your account'
  return <main className="flex min-h-svh items-center justify-center bg-background px-6 py-12">
    <section className="flex w-full max-w-sm flex-col gap-6" aria-labelledby="student-register-heading">
      <header className="flex flex-col items-center gap-3 text-center">
        <Link to="/" aria-label="Ferrix home" title="Ferrix home" className="mb-3 flex size-12 items-center justify-center rounded-xl border bg-card outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-4 focus-visible:ring-offset-background"><Layers3 className="size-6" aria-hidden="true" /></Link>
        <h1 id="student-register-heading" className="text-2xl font-semibold tracking-tight">{title}</h1>
        <p className="text-sm text-muted-foreground">{submitted ? 'Sign in to check your hosting status.' : 'Your PHP website and MariaDB database start here.'}</p>
      </header>
      {message ? <Alert variant="destructive" role="alert"><AlertTitle>Unable to continue</AlertTitle><AlertDescription>{message}</AlertDescription></Alert> : null}
      {submitted ? <>
        <Alert role="status"><AlertTitle>Waiting for approval</AlertTitle><AlertDescription>New accounts need administrator approval before hosting is available. If you already have an account, sign in as usual.</AlertDescription></Alert>
        <Button asChild><Link to="/login">Sign in</Link></Button>
      </> : open === false ? <>
        <p className="text-center text-sm text-muted-foreground">Student registration is currently closed. Contact your teacher.</p>
        <Button asChild variant="outline"><Link to="/login">Sign in</Link></Button>
      </> : open === null ? <div className="flex flex-col items-center gap-3">
        {message ? <Button variant="outline" onClick={checkRegistration}>Try again</Button> : <p role="status" className="text-sm text-muted-foreground">Checking registration…</p>}
      </div> : <>
        <form onSubmit={submit} className="flex flex-col gap-6">
          <fieldset disabled={busy}><FieldGroup>
            <Field><FieldLabel htmlFor="student-name">Name</FieldLabel><Input id="student-name" name="name" required maxLength={80} autoComplete="name" /></Field>
            <Field><FieldLabel htmlFor="student-email">Email</FieldLabel><Input id="student-email" name="email" type="email" required maxLength={254} autoComplete="email" /></Field>
            <Field><FieldLabel htmlFor="student-password">Password</FieldLabel>
              <InputGroup>
                <InputGroupInput id="student-password" name="password" type={showPassword ? 'text' : 'password'} required minLength={12} maxLength={1024} autoComplete="new-password" aria-describedby="student-password-hint" />
                <InputGroupAddon align="inline-end"><InputGroupButton type="button" size="icon-sm" aria-label={showPassword ? 'Hide password' : 'Show password'} aria-pressed={showPassword} onClick={() => setShowPassword(!showPassword)}>{showPassword ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon>
              </InputGroup>
              <FieldDescription id="student-password-hint">Use at least 12 characters.</FieldDescription>
            </Field>
          </FieldGroup></fieldset>
          <p className="text-sm leading-relaxed text-muted-foreground">Your administrator will review your registration before your hosting is activated.</p>
          <Button type="submit" className="w-full" disabled={busy}>{busy ? 'Submitting…' : 'Register as student'}</Button>
        </form>
        <p className="text-center text-sm text-muted-foreground">Already have an account? <Link to="/login" className="font-medium text-foreground underline underline-offset-4">Sign in</Link></p>
      </>}
    </section>
  </main>
}
export function StudentHostingPage({identity}:{identity:Identity}){
 const admin=identity.system_role==='admin';const [rows,setRows]=useState<Hosting[]>([]),[settings,setSettings]=useState<Settings|null>(null),[selected,setSelected]=useState<Hosting|null>(null),[message,setMessage]=useState(''),[busy,setBusy]=useState(false),[showPassword,setShowPassword]=useState(false),[loading,setLoading]=useState(true)
 async function refresh(){try{const data=await request<Hosting[]>('/student-hosting');setRows(data);if(admin){setSettings(await request<Settings>('/student-hosting/settings'));if(selected)setSelected(await request<Hosting>(`/student-hosting/${selected.user_id}`));}else if(data[0])setSelected(await request<Hosting>(`/student-hosting/${identity.user_id}`));}catch(e){setMessage(errorText(e))}finally{setLoading(false)}}
 useEffect(()=>{void refresh();const timer=window.setInterval(()=>{if(document.visibilityState==='visible')void refresh()},5000);return()=>clearInterval(timer)},[identity.user_id,admin,selected?.user_id])
 async function action(path:string,body:unknown={},notice='Saved.'){setBusy(true);setMessage('');try{await request(path,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body),signal:AbortSignal.timeout(65000)});setMessage(notice);await refresh();if(admin&&selected)setSelected(await request<Hosting>(`/student-hosting/${selected.user_id}`));}catch(e){setMessage(errorText(e))}finally{setBusy(false)}}
 async function select(row:Hosting){setShowPassword(false);try{setSelected(await request<Hosting>(`/student-hosting/${row.user_id}`))}catch(e){setMessage(errorText(e))}}
 async function upload(event:FormEvent<HTMLFormElement>){event.preventDefault();const input=event.currentTarget.elements.namedItem('files') as HTMLInputElement;const files=Array.from(input.files??[]);if(!selected||!files.length)return;const total=files.reduce((n,f)=>n+f.size,0);if(total>8*1024*1024||files.length>256){setMessage('Choose up to 256 files and 8 MiB total.');return}setBusy(true);try{const paths=files.map(f=>f.webkitRelativePath||f.name);const root=paths[0]?.split('/')[0];const payload=await Promise.all(files.map(async(f,i)=>({path:f.webkitRelativePath?paths[i].slice((root?.length??0)+1):f.name,hex:Array.from(new Uint8Array(await f.arrayBuffer()),b=>b.toString(16).padStart(2,'0')).join('')})));await action(`/student-hosting/${selected.user_id}/upload`,{files:payload},'Files uploaded. Open your website to see the changes.')}catch(e){setMessage(errorText(e))}finally{setBusy(false)}}
 return <div className="flex flex-col gap-6"><div><h1 className="text-2xl font-semibold">Hosting</h1><p className="text-sm text-muted-foreground">{admin?'Approve students and manage their PHP websites on one shared MariaDB server.':'Your PHP website and private database.'}</p></div><Message message={message}/>{admin&&settings?<Card><CardHeader><CardTitle><h2>Shared PHP-FPM &amp; MariaDB</h2></CardTitle><CardDescription>One PHP runtime and one MariaDB server, with separate files, PHP pools and database users for each student.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4"><p>Server: <Badge variant="secondary">{settings.server_state}</Badge></p>{settings.error?<p role="alert">{settings.error}</p>:null}<p className="text-sm text-muted-foreground">Registration is {settings.registration_open?'open':'closed'}. New students wait for approval.</p><div className="flex flex-wrap gap-2"><Button disabled={busy} onClick={()=>action('/student-hosting/settings',{registration_open:!settings.registration_open})}>{!settings.enabled?'Enable Hosting':settings.registration_open?'Close registration':'Open registration'}</Button><Button asChild variant="outline"><Link to="/student-register">Student registration page</Link></Button></div></CardContent></Card>:null}
 {admin?<Card><CardHeader><CardTitle><h2>Students</h2></CardTitle><CardDescription>Approve a registration to create its website and database. Up to 500 registrations are displayed.</CardDescription></CardHeader><CardContent><DataTable label="Students" data={rows.map(row=>({...row,id:row.user_id}))} columns={[{id:'student',label:'Student',value:r=>`${r.name} ${r.email}`,cell:r=><div>{r.name}<p className="text-xs text-muted-foreground">{r.email}</p></div>},{id:'status',label:'Status',value:r=>r.status,cell:r=><Badge variant="outline">{r.status}</Badge>},{id:'actions',label:'Actions',cell:r=><div className="flex flex-wrap gap-2"><Button variant="outline" disabled={busy} onClick={()=>select(r)}>Manage</Button>{['pending','failed','suspended'].includes(r.status)?<Button disabled={busy} onClick={()=>action(`/student-hosting/${r.user_id}/approve`,{},'Provisioning queued.')}>{r.status==='pending'?'Approve':r.status==='suspended'?'Resume':'Retry'}</Button>:null}{r.status==='pending'?<Button variant="outline" disabled={busy} onClick={()=>action(`/student-hosting/${r.user_id}/reject`)}>Reject</Button>:null}{r.status==='active'?<Button variant="outline" disabled={busy} onClick={()=>action(`/student-hosting/${r.user_id}/suspend`)}>Suspend</Button>:null}</div>}]} /></CardContent></Card>:null}
 {loading?<p role="status">Loading hosting…</p>:null}{selected?<><Card><CardHeader><CardTitle><h2>{selected.name}</h2></CardTitle><CardDescription>Status: {selected.status}</CardDescription></CardHeader><CardContent className="flex flex-col gap-4">{selected.error?<p role="alert">{selected.error}</p>:null}{selected.status==='pending'?<p>Your registration is waiting for your teacher’s approval.</p>:null}{['queued','provisioning'].includes(selected.status)?<p role="status">Your PHP website and database are being prepared.</p>:null}{selected.status==='rejected'?<p>Your registration was not approved. Contact your teacher.</p>:null}{selected.status==='suspended'?<p>Your hosting is suspended. Contact your teacher.</p>:null}{selected.status==='active'&&selected.automatic_url?<><p className="break-all text-sm">{selected.automatic_url}</p><Button asChild className="self-start"><a href={selected.automatic_url} target="_blank" rel="noopener noreferrer">Open website<ExternalLink data-icon="inline-end"/></a></Button><p className="text-sm text-muted-foreground">Your website is public. Keep database passwords out of downloadable files.</p></>:null}</CardContent></Card>
 {selected.status==='active'?<><HostingTools key={selected.user_id} userId={selected.user_id} database={selected.database_name}/><Card><CardHeader><CardTitle><h2>Database connection</h2></CardTitle><CardDescription>Use PDO or mysqli in PHP. This account can access only your database.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4"><dl className="grid gap-3 sm:grid-cols-2">{[['Host',selected.database_host],['Port',String(selected.database_port)],['Database',selected.database_name],['User',selected.database_user]].map(([label,value])=><div key={label}><dt className="text-sm text-muted-foreground">{label}</dt><dd className="break-all font-mono text-sm">{value}</dd></div>)}</dl><Field><FieldLabel htmlFor="hosting-password">Database password</FieldLabel><Input id="hosting-password" type={showPassword?'text':'password'} value={selected.database_password??''} readOnly autoComplete="off"/></Field><Button variant="outline" className="self-start" onClick={()=>setShowPassword(!showPassword)}>{showPassword?'Hide password':'Show password'}</Button><pre className="overflow-x-auto rounded-md bg-muted p-4 text-xs">{'$pdo = new PDO(\n  "mysql:host=" . getenv("DB_HOST") .\n  ";dbname=" . getenv("DB_NAME") . ";charset=utf8mb4",\n  getenv("DB_USER"), getenv("DB_PASSWORD")\n);'}</pre></CardContent></Card><Card><CardHeader><CardTitle><h2>Upload PHP project</h2></CardTitle><CardDescription>Choose your project folder. Existing files with the same paths are replaced; other files are retained. Up to 256 files and 8 MiB per upload.</CardDescription></CardHeader><CardContent><form onSubmit={upload} className="flex flex-col gap-4"><Field><FieldLabel htmlFor="hosting-files">Project folder</FieldLabel><Input id="hosting-files" name="files" type="file" multiple {...{webkitdirectory:''}} required disabled={busy}/></Field><Button className="self-start" disabled={busy}>{busy?'Uploading…':'Upload project'}</Button></form></CardContent></Card></>:null}</>:!admin&&!loading?<p>No hosting account found.</p>:null}</div>
}
