import { CirclePlus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuLabel, DropdownMenuSeparator } from '@/components/ui/dropdown-menu'

export function DataTableFilter({ label, value, onChange, options }: { label: string; value: string; onChange: (value: string) => void; options: [string, string][] }) {
  const selected = options.find(([key]) => key === value)?.[1]
  return <DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline" size="sm" className="h-8 gap-2 border-dashed" aria-label={`Filter ${label.toLowerCase()}: ${selected}`}><CirclePlus aria-hidden="true" /><span>{label}</span>{value ? <Badge variant="secondary" className="rounded-sm px-1.5 text-xs">{selected}</Badge> : null}</Button></DropdownMenuTrigger><DropdownMenuContent align="start"><DropdownMenuLabel>{label}</DropdownMenuLabel><DropdownMenuSeparator /><DropdownMenuRadioGroup value={value} onValueChange={onChange}>{options.map(([key, text]) => <DropdownMenuRadioItem key={key} value={key}>{text}</DropdownMenuRadioItem>)}</DropdownMenuRadioGroup></DropdownMenuContent></DropdownMenu>
}
