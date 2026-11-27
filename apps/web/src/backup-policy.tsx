import { LoadingButton } from '@/components/loading-button'
import { useEffect, useRef, useState } from 'react'
import { request } from '@/lib/api'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Button } from '@/components/ui/button'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
type Destination = { endpoint:string;region:string;bucket:string;prefix:string;path_style:boolean;credentials_saved:boolean }
type Policy = { application_offline?:boolean;enabled:boolean;cron:string;timezone:string;keep_last:number;s3:Destination|null;next_run_at:string|null;last_run_at:string|null;last_message:string|null }
export function BackupPolicy({serviceId,application=false}:{serviceId:string;application?:boolean}) {
 const [policy,setPolicy]=useState<Policy|null>(null)
 const [error,setError]=useState('')
 const [version,setVersion]=useState(0)
 useEffect(()=>{const abort=new AbortController();setError('');setPolicy(null);request<Policy>(`/services/${serviceId}/backup-policy`,{signal:abort.signal}).then(p=>{if(!abort.signal.aborted)setPolicy(p)}).catch(e=>{if(!abort.signal.aborted)setError(e.message)});return()=>abort.abort()},[serviceId,version])
 return <Card><CardHeader><CardTitle><h2>Schedule and storage</h2></CardTitle><CardDescription>{application ? 'Scheduled offline Application backups with optional S3-compatible storage. Each run may cause downtime.' : 'Automatic PostgreSQL and MariaDB backups with optional S3-compatible storage.'}</CardDescription></CardHeader><CardContent>{error ? <div className="flex flex-col gap-3"><Alert variant="destructive"><AlertTitle>Could not load backup settings</AlertTitle><AlertDescription>{error}</AlertDescription></Alert><Button variant="outline" onClick={()=>setVersion(v=>v+1)}>Retry settings</Button></div> : policy ? <PolicyForm key={`${serviceId}-${version}`} serviceId={serviceId} initial={policy} application={application} /> : <Skeleton className="h-48" />}</CardContent></Card>
}
function PolicyForm({serviceId,initial,application}:{serviceId:string;initial:Policy;application:boolean}) {
 const [policy,setPolicy]=useState(initial)
 const [enabled,setEnabled]=useState(initial.enabled)
 const [offline,setOffline]=useState(initial.application_offline??false)
 const [storage,setStorage]=useState(initial.s3?'s3':'local')
 const [replace,setReplace]=useState(false)
 const [pathStyle,setPathStyle]=useState(initial.s3?.path_style??true)
 const [busy,setBusy]=useState(false)
 const [dirty,setDirty]=useState(false)
 const [message,setMessage]=useState('')
 const [error,setError]=useState('')
 const form=useRef<HTMLFormElement>(null)
 const base=`/services/${serviceId}/backup-policy`
 useEffect(()=>{const abort=new AbortController();let running=false;async function refresh(){if(document.hidden||running)return;running=true;try{const p=await request<Policy>(base,{signal:abort.signal});if(!abort.signal.aborted)setPolicy(p)}catch{/* Saving and Test connection report actionable errors. */}finally{running=false}}const timer=setInterval(()=>void refresh(),30_000);return()=>{abort.abort();clearInterval(timer)}},[base])
 const credentialsNeeded=replace||!policy.s3?.credentials_saved
 return <form ref={form} className="flex flex-col gap-4" onInput={()=>setDirty(true)} onSubmit={async event=>{
  event.preventDefault();const data=new FormData(event.currentTarget);setBusy(true);setMessage('');setError('')
  const s3=storage==='s3'?{endpoint:data.get('endpoint'),region:data.get('region'),bucket:data.get('bucket'),prefix:data.get('prefix'),path_style:pathStyle,credentials:credentialsNeeded?{access_key:data.get('access_key'),secret_key:data.get('secret_key'),session_token:data.get('session_token')||null}:null}:null
  try{await request(base,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({application_offline:offline,enabled,cron:data.get('cron'),timezone:data.get('timezone'),keep_last:Number(data.get('keep_last')),s3})});setPolicy(await request<Policy>(base));setDirty(false);setReplace(false);for(const name of ['access_key','secret_key','session_token']){const input=form.current?.elements.namedItem(name);if(input instanceof HTMLInputElement)input.value=''}setMessage('Backup schedule and storage saved.')}
  catch(e){setError(e instanceof Error?e.message:'Could not save backup settings.')}finally{setBusy(false)}
 }}>
 {error ? <Alert variant="destructive"><AlertTitle>Unable to continue</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}
 {message ? <p role="status" className="text-sm">{message}</p> : null}
 <fieldset disabled={busy}><FieldGroup>
  <Field orientation="horizontal"><Checkbox id={`schedule-${serviceId}`} checked={enabled} onCheckedChange={value=>{setEnabled(value===true);setDirty(true)}} /><FieldLabel htmlFor={`schedule-${serviceId}`}>Enable scheduled backups</FieldLabel></Field>
  {application ? <Field orientation="horizontal"><Checkbox id={`offline-${serviceId}`} checked={offline} onCheckedChange={v=>{setOffline(v===true);setDirty(true)}} required={enabled} /><FieldLabel htmlFor={`offline-${serviceId}`}>I understand scheduled backups stop this service temporarily and restore its previous runtime state.</FieldLabel></Field> : null}
  <div className="grid gap-4 md:grid-cols-2"><Field><FieldLabel htmlFor={`cron-${serviceId}`}>Cron expression</FieldLabel><Input id={`cron-${serviceId}`} name="cron" defaultValue={initial.cron} required maxLength={128} placeholder="0 2 * * *" /><FieldDescription>Five fields: minute, hour, day, month, weekday. For example, 0 2 * * * runs daily at 02:00.</FieldDescription></Field><Field><FieldLabel htmlFor={`timezone-${serviceId}`}>Timezone</FieldLabel><Input id={`timezone-${serviceId}`} name="timezone" defaultValue={initial.timezone} required placeholder="Asia/Kuala_Lumpur" /><FieldDescription>Use an IANA timezone. Missed runs are combined into one backup after restart.</FieldDescription></Field></div>
  <div className="grid gap-4 md:grid-cols-2"><Field><FieldLabel htmlFor={`storage-${serviceId}`}>Backup storage</FieldLabel><NativeSelect id={`storage-${serviceId}`} value={storage} onChange={e=>{setStorage(e.target.value);setDirty(true)}}><NativeSelectOption value="local">Docker server only</NativeSelectOption><NativeSelectOption value="s3">Docker server and S3</NativeSelectOption></NativeSelect><FieldDescription>Backup now uses this destination even when the schedule is disabled.</FieldDescription></Field><Field><FieldLabel htmlFor={`keep-${serviceId}`}>Successful backups to keep</FieldLabel><Input id={`keep-${serviceId}`} name="keep_last" type="number" min={1} max={365} required defaultValue={initial.keep_last} /><FieldDescription>After a new backup succeeds, older backups are removed from local storage and their original S3 destination. Backups being restored are protected.</FieldDescription></Field></div>
  {storage==='s3'?<>
   <div className="grid gap-4 md:grid-cols-2"><Field><FieldLabel htmlFor={`endpoint-${serviceId}`}>S3 endpoint</FieldLabel><Input id={`endpoint-${serviceId}`} name="endpoint" type="url" defaultValue={initial.s3?.endpoint??''} placeholder="https://s3.example.com" /><FieldDescription>Leave blank for AWS S3. Use HTTPS for remote storage.</FieldDescription></Field><Field><FieldLabel htmlFor={`region-${serviceId}`}>Region</FieldLabel><Input id={`region-${serviceId}`} name="region" defaultValue={initial.s3?.region??'us-east-1'} required /></Field><Field><FieldLabel htmlFor={`bucket-${serviceId}`}>Bucket</FieldLabel><Input id={`bucket-${serviceId}`} name="bucket" defaultValue={initial.s3?.bucket??''} required maxLength={63} /></Field><Field><FieldLabel htmlFor={`prefix-${serviceId}`}>Object prefix</FieldLabel><Input id={`prefix-${serviceId}`} name="prefix" defaultValue={initial.s3?.prefix??'ferrix-backups'} maxLength={256} /><FieldDescription>Ferrix adds unique service and backup IDs beneath this prefix.</FieldDescription></Field></div>
   <Field orientation="horizontal"><Checkbox id={`path-${serviceId}`} checked={pathStyle} onCheckedChange={v=>{setPathStyle(v===true);setDirty(true)}} /><FieldLabel htmlFor={`path-${serviceId}`}>Use path-style addressing</FieldLabel></Field>
   {policy.s3?.credentials_saved?<Field orientation="horizontal"><Checkbox id={`replace-${serviceId}`} checked={replace} onCheckedChange={v=>{setReplace(v===true);setDirty(true)}} /><FieldLabel htmlFor={`replace-${serviceId}`}>Replace saved S3 credentials</FieldLabel></Field>:null}
   {credentialsNeeded?<div className="grid gap-4 md:grid-cols-2"><Field><FieldLabel htmlFor={`access-${serviceId}`}>Access key ID</FieldLabel><Input id={`access-${serviceId}`} name="access_key" required autoComplete="off" maxLength={256} /></Field><Field><FieldLabel htmlFor={`secret-${serviceId}`}>Secret access key</FieldLabel><Input id={`secret-${serviceId}`} name="secret_key" type="password" required autoComplete="new-password" maxLength={4096} /></Field><Field className="md:col-span-2"><FieldLabel htmlFor={`token-${serviceId}`}>Session token (optional)</FieldLabel><Input id={`token-${serviceId}`} name="session_token" type="password" autoComplete="off" maxLength={8192} /></Field></div>:<p className="text-sm text-muted-foreground">S3 credentials are saved and encrypted. Their values are never returned to the browser.</p>}
  </>:null}
  <div className="flex flex-wrap gap-2"><LoadingButton loading={busy} loadingText="Saving…" type="submit" disabled={busy}>{busy?'Working…':'Save backup settings'}</LoadingButton>{storage==='s3'?<Button type="button" variant="outline" disabled={busy||dirty||!policy.s3} onClick={async()=>{setBusy(true);setError('');setMessage('');try{const result=await request<{message:string}>(`${base}/test`,{method:'POST',headers:{'Content-Type':'application/json'},body:'{}',signal:AbortSignal.timeout(35_000)});setMessage(result.message)}catch(e){setError(e instanceof Error?e.message:'S3 connection test failed.')}finally{setBusy(false)}}}>Test connection</Button>:null}</div>
 </FieldGroup></fieldset>
 <p className="text-sm text-muted-foreground">Next run: {policy.enabled&&policy.next_run_at?new Date(policy.next_run_at).toLocaleString():'Schedule disabled'}{policy.last_message?` · ${policy.last_message}`:''}</p>
 </form>
}
