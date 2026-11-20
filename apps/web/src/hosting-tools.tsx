import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { ArrowUp, Download, File, Folder, Plus, RefreshCw, Upload } from 'lucide-react'
import { request } from '@/lib/api'
import { DataTable } from '@/components/data-table'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'

type Entry = { name: string; directory: boolean; size: number }
type Result = { entries?: Entry[]; hex?: string }
type Editor = { mode: 'write' | 'mkdir' | 'rename' | 'delete'; path: string; name: string; text: string }
const encode = (bytes: Uint8Array) => Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('')
const decode = (hex: string) => Uint8Array.from(hex.match(/../g) ?? [], b => parseInt(b, 16))
const errorText = (error: unknown) => error instanceof Error ? error.message : 'Unable to continue. Please retry.'
function download(hex: string, name: string, type: string) {
  const url = URL.createObjectURL(new Blob([decode(hex)], { type }))
  const link = document.createElement('a'); link.href = url; link.download = name; link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

export function HostingTools({ userId, database }: { userId: string; database: string }) {
  const [path, setPath] = useState('')
  const [entries, setEntries] = useState<Entry[]>([])
  const [busy, setBusy] = useState(false)
  const [loading, setLoading] = useState(true)
  const [notice, setNotice] = useState('')
  const [error, setError] = useState('')
  const [editor, setEditor] = useState<Editor | null>(null)
  const [sqlFile, setSqlFile] = useState<globalThis.File | null>(null)
  const [confirmImport, setConfirmImport] = useState(false)
  const prefix = `/student-hosting/${userId}`
  const fullPath = (name: string) => path ? `${path}/${name}` : name
  async function call(kind: 'files' | 'database', body: object) {
    return request<Result>(`${prefix}/${kind}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(65000) })
  }
  async function refresh() {
    setLoading(true)
    try { const result = await call('files', { operation: 'list', path }); setEntries(result.entries ?? []) }
    catch (e) { setEntries([]); setError(errorText(e)) }
    finally { setLoading(false) }
  }
  useEffect(() => {
    let cancelled = false
    setLoading(true); setEntries([]); setError('')
    void call('files', { operation: 'list', path }).then(result => { if (!cancelled) setEntries(result.entries ?? []) }).catch(e => { if (!cancelled) setError(errorText(e)) }).finally(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [userId, path])
  async function perform(action: () => Promise<void>) {
    setBusy(true); setError(''); setNotice('')
    try { await action() } catch (e) { setError(errorText(e)) } finally { setBusy(false) }
  }
  async function read(entry: Entry, edit: boolean) {
    await perform(async () => {
      if (entry.size > 8 * 1024 * 1024) throw new Error('Choose a file up to 8 MiB.')
      const result = await call('files', { operation: 'read', path: fullPath(entry.name) })
      if (edit) {
        if (entry.size > 1024 * 1024) throw new Error('The text editor supports files up to 1 MiB. Download larger files instead.')
        let text: string
        try { text = new TextDecoder('utf-8', { fatal: true }).decode(decode(result.hex ?? '')) } catch { throw new Error('This is a binary file. Download it instead.') }
        if (text.includes('\0')) throw new Error('This is a binary file. Download it instead.')
        setEditor({ mode: 'write', path: fullPath(entry.name), name: entry.name, text })
      } else download(result.hex ?? '', entry.name, 'application/octet-stream')
    })
  }
  async function save(event: FormEvent) {
    event.preventDefault()
    if (!editor) return
    await perform(async () => {
      const target = editor.path || fullPath(editor.name)
      const bytes = new TextEncoder().encode(editor.text)
      if (editor.mode === 'write' && bytes.length > 1024 * 1024) throw new Error('Text files in the editor must be up to 1 MiB.')
      await call('files', { operation: editor.mode, path: target, ...(editor.mode === 'write' ? { hex: encode(bytes) } : editor.mode === 'rename' ? { name: editor.name } : {}) })
      setEditor(null); setNotice('Files updated.'); await refresh()
    })
  }
  async function upload(files: FileList | null) {
    const list = Array.from(files ?? [])
    if (!list.length) return
    await perform(async () => {
      if (list.length > 256 || list.reduce((n, file) => n + file.size, 0) > 8 * 1024 * 1024) throw new Error('Choose up to 256 files and 8 MiB total.')
      for (const file of list) await call('files', { operation: 'write', path: fullPath(file.name), hex: encode(new Uint8Array(await file.arrayBuffer())) })
      setNotice('Files uploaded.'); await refresh()
    })
  }
  return <>
    <Card><CardHeader><CardTitle><h2>File Manager</h2></CardTitle><CardDescription>Manage your website files in public_html. Uploads and saved files replace existing content. Files up to 8 MiB; text editor up to 1 MiB.</CardDescription></CardHeader>
      <CardContent className="flex min-w-0 flex-col gap-4">
        {error ? <Alert variant="destructive" role="alert"><AlertTitle>Unable to continue</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}
        {notice ? <p role="status" className="text-sm text-muted-foreground">{notice}</p> : null}
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" disabled={busy || loading || !path} onClick={() => setPath(path.split('/').slice(0, -1).join('/'))}><ArrowUp data-icon="inline-start" />Up</Button>
          <Button variant="outline" disabled={busy || loading} onClick={() => void refresh()}><RefreshCw data-icon="inline-start" />Refresh</Button>
          <Button variant="outline" disabled={busy || loading} onClick={() => setEditor({ mode: 'write', path: '', name: '', text: '' })}><Plus data-icon="inline-start" />New file</Button>
          <Button variant="outline" disabled={busy || loading} onClick={() => setEditor({ mode: 'mkdir', path: '', name: '', text: '' })}><Folder data-icon="inline-start" />New folder</Button>
        </div>
        <p className="break-all font-mono text-sm" aria-label="Current folder">public_html/{path}</p>
        <Field><FieldLabel htmlFor="hosting-upload-files"><Upload className="size-4" aria-hidden="true" />Upload files into this folder</FieldLabel><Input id="hosting-upload-files" type="file" multiple disabled={busy || loading} onChange={e => { void upload(e.target.files); e.target.value = '' }} /></Field>
        {loading ? <p role="status">Loading files…</p> : <DataTable key={path} label="Files" data={entries.map(entry => ({ ...entry, id: entry.name }))} columns={[
          { id: 'name', label: 'Name', value: e => e.name, cell: e => <Button variant="ghost" disabled={busy} onClick={() => e.directory ? setPath(fullPath(e.name)) : void read(e, true)}>{e.directory ? <Folder data-icon="inline-start" /> : <File data-icon="inline-start" />}<span className="max-w-56 truncate">{e.name}</span></Button> },
          { id: 'size', label: 'Size', value: e => e.directory ? 'Folder' : `${e.size.toLocaleString()} bytes` },
          { id: 'actions', label: 'Actions', cell: e => <div className="flex flex-wrap gap-2">
            {!e.directory ? <Button variant="outline" disabled={busy} onClick={() => void read(e, false)} aria-label={`Download ${e.name}`}><Download /></Button> : null}
            <Button variant="outline" disabled={busy} onClick={() => setEditor({ mode: 'rename', path: fullPath(e.name), name: e.name, text: '' })}>Rename</Button>
            <Button variant="outline" disabled={busy} onClick={() => setEditor({ mode: 'delete', path: fullPath(e.name), name: e.name, text: '' })}>Delete</Button>
          </div> },
        ]} />}
      </CardContent>
    </Card>
    <Card><CardHeader><CardTitle><h2>Import / export database</h2></CardTitle><CardDescription>Only {database} is accessible. Exports include tables and data, up to 16 MiB; views, triggers and routines are excluded.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4">
      <Button className="self-start" variant="outline" disabled={busy} onClick={() => void perform(async () => { const result = await call('database', { operation: 'export' }); download(result.hex ?? '', `${database}.sql`, 'application/sql'); setNotice('Database export downloaded.') })}><Download data-icon="inline-start" />Export SQL</Button>
      <Field><FieldLabel htmlFor="hosting-sql">SQL file to import (up to 8 MiB)</FieldLabel><Input id="hosting-sql" type="file" accept=".sql,application/sql,text/plain" disabled={busy} onChange={e => setSqlFile(e.target.files?.[0] ?? null)} /></Field>
      <p className="text-sm text-muted-foreground">Import may replace existing tables. Export a backup first. If an import fails, earlier statements may already have been applied.</p>
      <Button className="self-start" disabled={busy || !sqlFile} onClick={() => { if (sqlFile && sqlFile.size > 8 * 1024 * 1024) setError('Choose a SQL file up to 8 MiB.'); else setConfirmImport(true) }}><Upload data-icon="inline-start" />Import SQL</Button>
    </CardContent></Card>
    <Dialog open={editor !== null} onOpenChange={open => { if (!open && !busy) setEditor(null) }}><DialogContent className="sm:max-w-2xl"><DialogHeader><DialogTitle>{editor?.mode === 'delete' ? 'Delete item?' : editor?.mode === 'rename' ? 'Rename item' : editor?.mode === 'mkdir' ? 'New folder' : 'Edit file'}</DialogTitle><DialogDescription>{editor?.mode === 'delete' ? 'This permanently deletes the item. Folders must be empty.' : 'Use letters, numbers, hyphens, underscores and dots. Changes are saved directly to your website.'}</DialogDescription></DialogHeader>
      <form onSubmit={save} className="flex min-w-0 flex-col gap-4"><FieldGroup>
        {editor?.mode === 'delete' ? <p className="break-all">{editor.path}</p> : <Field><FieldLabel htmlFor="hosting-item-name">Name</FieldLabel><Input id="hosting-item-name" value={editor?.name ?? ''} readOnly={!!editor?.path && editor.mode === 'write'} required maxLength={100} pattern="[A-Za-z0-9_][A-Za-z0-9_.-]*" disabled={busy} onChange={e => setEditor(current => current ? { ...current, name: e.target.value } : null)} /></Field>}
        {editor?.mode === 'write' ? <Field><FieldLabel htmlFor="hosting-file-content">File content</FieldLabel><Textarea id="hosting-file-content" rows={14} className="max-h-[50svh] font-mono" spellCheck={false} value={editor.text} disabled={busy} onChange={e => setEditor(current => current ? { ...current, text: e.target.value } : null)} /></Field> : null}
      </FieldGroup>{error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}<DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={() => setEditor(null)}>Cancel</Button><Button type="submit" variant={editor?.mode === 'delete' ? 'destructive' : 'default'} disabled={busy}>{busy ? 'Saving…' : editor?.mode === 'delete' ? 'Delete' : 'Save'}</Button></DialogFooter></form>
    </DialogContent></Dialog>
    <Dialog open={confirmImport} onOpenChange={open => { if (!busy) setConfirmImport(open) }}><DialogContent><DialogHeader><DialogTitle>Import SQL?</DialogTitle><DialogDescription>This runs {sqlFile?.name} against your database. It can replace tables and data. Download an export before proceeding.</DialogDescription></DialogHeader>{error ? <p role="alert">{error}</p> : null}<DialogFooter><Button variant="outline" disabled={busy} onClick={() => setConfirmImport(false)}>Cancel</Button><Button disabled={busy} onClick={() => void perform(async () => { if (!sqlFile || sqlFile.size > 8 * 1024 * 1024) throw new Error('Choose a SQL file up to 8 MiB.'); await call('database', { operation: 'import', sql: new TextDecoder('utf-8', { fatal: true }).decode(await sqlFile.arrayBuffer()) }); setConfirmImport(false); setNotice('Database imported.'); })}>{busy ? 'Importing…' : 'Import SQL'}</Button></DialogFooter></DialogContent></Dialog>
  </>
}
