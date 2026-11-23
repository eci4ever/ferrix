import { useEffect, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { Bell, CheckCheck, RefreshCw } from 'lucide-react'
import { request } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Sheet, SheetTrigger, SheetContent, SheetHeader, SheetTitle, SheetDescription } from '@/components/ui/sheet'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Empty, EmptyHeader, EmptyTitle, EmptyDescription } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
type Notification = { id: string; project_id: string; service_id: string | null; kind: string; title: string; message: string; created_at: string; read: boolean }
type Inbox = { notifications: Notification[]; unread: number }
export function Notifications() {
 const [open,setOpen] = useState(false)
 const [inbox,setInbox] = useState<Inbox | null>(null)
 const [error,setError] = useState('')
 const [version,setVersion] = useState(0)
 const [busy,setBusy] = useState(false)
 useEffect(() => {
  const abort = new AbortController(); let timer: ReturnType<typeof setTimeout>
  async function poll() {
   if (!document.hidden) try { const result = await request<Inbox>('/notifications', { signal: abort.signal }); if (!Array.isArray(result.notifications) || !Number.isFinite(result.unread)) throw new Error('Invalid notification response. Refresh to try again.'); if (!abort.signal.aborted) { setInbox(result); setError('') } } catch (error) { if (!abort.signal.aborted) setError(error instanceof Error ? error.message : 'Could not load notifications.') }
   if (!abort.signal.aborted) timer = setTimeout(poll, 15_000)
  }
  function visible() { if (!document.hidden) { clearTimeout(timer); void poll() } }
  void poll(); document.addEventListener('visibilitychange',visible)
  return () => { abort.abort(); clearTimeout(timer); document.removeEventListener('visibilitychange',visible) }
 }, [version])
 async function mark(id?: string) {
  setBusy(true); setError('')
  try { await request(id ? `/notifications/${id}/read` : '/notifications/read-all',{method:'POST',headers:{'Content-Type':'application/json'},body:'{}'}); setVersion(v=>v+1) }
  catch(error) { setError(error instanceof Error ? error.message : 'Could not mark notifications as read.') }
  finally { setBusy(false) }
 }
 return <Sheet open={open} onOpenChange={setOpen}><SheetTrigger asChild><Button variant="outline" size="icon" className="relative" aria-label={inbox?.unread ? `Notifications, ${inbox.unread} unread` : 'Notifications'}><Bell />{inbox?.unread ? <span className="sr-only">Unread notifications</span> : null}{inbox?.unread ? <Badge className="absolute -right-2 -top-2" variant="secondary">{inbox.unread > 99 ? '99+' : inbox.unread}</Badge> : null}</Button></SheetTrigger><SheetContent className="flex w-full flex-col gap-4 overflow-y-auto sm:max-w-lg"><SheetHeader><SheetTitle>Notifications</SheetTitle><SheetDescription>Deployment, backup and resource alerts for this organization. Latest 100 events, retained for 30 days. Resource alerts require at least 90% of a configured CPU or RAM limit for one minute.</SheetDescription></SheetHeader><div className="flex flex-col gap-4 px-4 pb-4"><div className="flex flex-wrap gap-2"><Button variant="outline" size="sm" disabled={busy || !inbox?.unread} onClick={()=>void mark()}><CheckCheck data-icon="inline-start" />Mark all as read</Button><Button variant="ghost" size="sm" onClick={()=>setVersion(v=>v+1)}><RefreshCw data-icon="inline-start" />Refresh</Button></div>{error ? <Alert variant="destructive"><AlertTitle>Could not load notifications</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}{!inbox && !error ? <Skeleton className="h-32" /> : inbox?.notifications.length ? <ul className="flex flex-col gap-3" aria-label="Notifications">{inbox.notifications.map(item=><li key={item.id}><Card><CardHeader><CardTitle className="flex flex-wrap items-center gap-2">{item.title}{!item.read ? <Badge variant="outline">Unread</Badge> : null}{item.kind.endsWith('_failed') || item.kind==='resource_high' ? <Badge variant="destructive">Attention</Badge> : null}</CardTitle><CardDescription className="break-words">{item.message}</CardDescription></CardHeader><CardContent className="flex flex-col gap-3"><time className="text-xs text-muted-foreground" dateTime={item.created_at}>{new Date(item.created_at).toLocaleString()}</time><div className="flex flex-wrap gap-2">{item.service_id ? <Button variant="outline" size="sm" asChild><Link to="/projects/$projectId/services/$serviceId" params={{projectId:item.project_id,serviceId:item.service_id}} onClick={()=>{ if(!item.read) void mark(item.id); setOpen(false) }}>View service</Link></Button> : null}{!item.read ? <Button variant="ghost" size="sm" disabled={busy} onClick={()=>void mark(item.id)}>Mark as read</Button> : null}</div></CardContent></Card></li>)}</ul> : !error ? <Empty><EmptyHeader><EmptyTitle>You're all caught up</EmptyTitle><EmptyDescription>New deployment, backup and resource alerts will appear here.</EmptyDescription></EmptyHeader></Empty> : null}</div></SheetContent></Sheet>
}
