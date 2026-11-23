import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'

type Configuration = { name: string; image: string; container_port: number; environment: string }

export function ConfigurationFields({ initial, disabled, source }: { initial?: Configuration; disabled: boolean; source?: string }) {
  return <FieldGroup>
    <Field><FieldLabel htmlFor="application-name">Application name</FieldLabel><Input id="application-name" name="name" required maxLength={80} autoFocus defaultValue={initial?.name ?? ''} placeholder="e.g. nginx" disabled={disabled} /></Field>
    {source === 'github' ? <Input type="hidden" name="image" value={initial?.image ?? 'nginx:alpine'} /> : <Field><FieldLabel htmlFor="application-image">Docker image</FieldLabel><Input id="application-image" name="image" required maxLength={255} defaultValue={initial?.image ?? 'nginx:alpine'} spellCheck={false} disabled={disabled} /><FieldDescription>Use a public registry image, for example nginx:alpine. GitHub sources are configured in the Source tab after creating the application.</FieldDescription></Field>}
    <Field><FieldLabel htmlFor="application-port">Container port</FieldLabel><Input id="application-port" name="container_port" type="number" min={1} max={65535} required defaultValue={initial?.container_port ?? 80} disabled={disabled} /><FieldDescription>Ferrix allocates a free port on the server's loopback address.</FieldDescription></Field>
    <Field><FieldLabel htmlFor="application-environment">Environment variables</FieldLabel><Textarea id="application-environment" name="environment" rows={5} maxLength={16384} defaultValue={initial?.environment ?? ''} placeholder={'KEY=value\nANOTHER_KEY=value'} spellCheck={false} disabled={disabled} /><FieldDescription>One KEY=value per line. Values are stored encrypted and passed literally; shell expressions are not expanded.</FieldDescription></Field>
  </FieldGroup>
}
export function configurationFrom(form: HTMLFormElement): Configuration {
  const data = new FormData(form)
  return { name: String(data.get('name') ?? ''), image: String(data.get('image') ?? ''), container_port: Number(data.get('container_port')), environment: String(data.get('environment') ?? '') }
}
