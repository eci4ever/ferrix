import { useIsMobile } from '@/hooks/use-mobile'
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from '@/components/ui/resizable'
import { Kbd } from '@/components/ui/kbd'
import { useEffect, useRef, useState } from 'react'
import { TerminalSquare, RotateCcw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogTrigger, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
export function ServiceTerminal({ serviceId, name, disabled }: {serviceId:string;name:string;disabled:boolean}) {
 const [open,setOpen]=useState(false)
 return <Dialog open={open} onOpenChange={setOpen}><DialogTrigger asChild><Button variant="outline" disabled={disabled}><TerminalSquare data-icon="inline-start" />Open terminal</Button></DialogTrigger><DialogContent className="flex max-h-[calc(100svh-2rem)] flex-col sm:max-w-5xl" onEscapeKeyDown={event=>event.preventDefault()} onInteractOutside={event=>event.preventDefault()}><DialogHeader><DialogTitle>Terminal · {name}</DialogTitle><DialogDescription>Runs a shell inside this service with its configured container user. Closes after 5 minutes without input or 30 minutes total. Closing disconnects this shell; files you change remain in the container.</DialogDescription></DialogHeader>{open ? <TerminalSession key={serviceId} serviceId={serviceId} /> : null}<DialogFooter><Button variant="outline" onClick={()=>setOpen(false)}>Close terminal</Button></DialogFooter></DialogContent></Dialog>
}
function TerminalSession({serviceId}:{serviceId:string}) {
 const mobile = useIsMobile()
 const host=useRef<HTMLDivElement>(null)
 const [status,setStatus]=useState('Connecting…')
 const [error,setError]=useState('')
 const [version,setVersion]=useState(0)
 useEffect(()=>{
  let disposed=false;let socket:WebSocket|undefined;let dispose:()=>void=()=>{}
  setStatus('Connecting…');setError('')
  async function connect(){
   try{
    const [{Terminal},{FitAddon}]=await Promise.all([import('@xterm/xterm'),import('@xterm/addon-fit'),import('@xterm/xterm/css/xterm.css')])
    if(disposed||!host.current)return
    const terminal=new Terminal({cursorBlink:true,scrollback:2000,fontSize:13,convertEol:false,screenReaderMode:true,allowProposedApi:false})
    const fit=new FitAddon();terminal.loadAddon(fit);terminal.open(host.current)
    terminal.textarea?.setAttribute('aria-label','Service terminal input')
    const color=(css:string)=>{const canvas=document.createElement('canvas');canvas.width=canvas.height=1;const ctx=canvas.getContext('2d')!;ctx.fillStyle=css;ctx.fillRect(0,0,1,1);const [r,g,b]=ctx.getImageData(0,0,1,1).data;return `#${[r,g,b].map(v=>v.toString(16).padStart(2,'0')).join('')}`}
    const theme=()=>{if(!host.current)return;const style=getComputedStyle(host.current);terminal.options.theme={background:color(style.backgroundColor),foreground:color(style.color),cursor:color(style.color)}}
    theme();const themeObserver=new MutationObserver(theme);themeObserver.observe(document.documentElement,{attributes:true,attributeFilter:['class']})
    let ready=false
    const url=new URL(`/api/v1/services/${serviceId}/terminal`,window.location.origin);url.protocol=location.protocol==='https:'?'wss:':'ws:'
    socket=new WebSocket(url);socket.binaryType='arraybuffer'
    const send=(message:object)=>{if(ready&&socket?.readyState===WebSocket.OPEN)socket.send(JSON.stringify(message))}
    const resize=()=>{if(disposed)return;try{fit.fit();send({type:'resize',cols:Math.min(300,Math.max(2,terminal.cols)),rows:Math.min(120,Math.max(2,terminal.rows))})}catch{/* Dialog may be between layout frames. */}}
    const observer=new ResizeObserver(resize);observer.observe(host.current)
    const input=terminal.onData(data=>{for(let i=0;i<data.length;i+=1024)send({type:'input',data:data.slice(i,i+1024)})})
    socket.onmessage=event=>{if(disposed)return;if(event.data instanceof ArrayBuffer){terminal.write(new Uint8Array(event.data));return}try{const message=JSON.parse(event.data);if(message.type==='ready'){ready=true;setStatus('Connected');resize();terminal.focus()}else if(message.type==='error'){setError(message.message);setStatus('Disconnected')}else if(message.type==='closed'){ready=false;setStatus('Disconnected')}}catch{setError('Received an invalid terminal response.')}}
    socket.onerror=()=>{if(!disposed){setError('Could not connect. Check your session and that this service is running with /bin/sh available.');setStatus('Disconnected')}}
    socket.onclose=()=>{ready=false;if(!disposed)setStatus('Disconnected')}
    dispose=()=>{observer.disconnect();themeObserver.disconnect();input.dispose();terminal.dispose()}
   }catch{if(!disposed){setError('Could not load the terminal. Try reconnecting.');setStatus('Disconnected')}}
  }
  void connect()
  return()=>{disposed=true;socket?.close();dispose()}
 },[serviceId,version])
 return <div className="flex min-h-0 flex-col gap-3"><div className="flex items-center justify-between gap-2"><Badge variant="outline" role="status">{status}</Badge><Button variant="ghost" size="sm" disabled={status!=='Disconnected'} onClick={()=>setVersion(v=>v+1)}><RotateCcw data-icon="inline-start" />Reconnect</Button></div>{error ? <Alert variant="destructive"><AlertTitle>Terminal disconnected</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}<ResizablePanelGroup orientation="vertical" disabled={mobile} className="h-[min(55svh,28rem)] min-h-64 rounded-md border"><ResizablePanel defaultSize="80%" minSize="140px"><div ref={host} className="h-full min-h-0 min-w-0 overflow-hidden rounded-md border bg-background p-2 text-foreground" aria-label="Interactive service terminal" /></ResizablePanel><ResizableHandle withHandle aria-label="Resize terminal and session help" className="h-3! sm:h-2!" /><ResizablePanel defaultSize="20%" minSize="64px"><div className="h-full overflow-auto bg-muted/30 p-3 text-xs text-muted-foreground"><p>Shell input runs inside this service. <Kbd>Ctrl</Kbd> + <Kbd>C</Kbd> interrupts a command.</p><p className="mt-1">{mobile ? 'Close terminal disconnects this shell.' : 'Drag the divider or use its arrow keys to resize. Close terminal disconnects this shell.'}</p></div></ResizablePanel></ResizablePanelGroup></div>
}
