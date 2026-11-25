import type { ReactNode } from 'react'
import { ChevronDown } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'

export function TelemetryCard({ title, description, collapsible = false, children }: { title: string; description: string; collapsible?: boolean; children: ReactNode }) {
  if (!collapsible) return <Card><CardHeader><CardTitle><h2>{title}</h2></CardTitle><CardDescription>{description}</CardDescription></CardHeader>{children}</Card>
  return <Collapsible defaultOpen={false} asChild><Card>
    <CardHeader><CardTitle><h2><CollapsibleTrigger asChild>
      <Button variant="ghost" className="group h-auto min-h-10 w-full justify-between gap-3 whitespace-normal text-left text-base">
        {title}<ChevronDown aria-hidden="true" className="shrink-0 transition-transform group-data-[state=open]:rotate-180 motion-reduce:transition-none" />
      </Button>
    </CollapsibleTrigger></h2></CardTitle></CardHeader>
    <CollapsibleContent forceMount className="data-[state=closed]:hidden">
      <div className="flex min-w-0 flex-col gap-4"><CardDescription className="px-(--card-spacing)">{description}</CardDescription>{children}</div>
    </CollapsibleContent>
  </Card></Collapsible>
}
