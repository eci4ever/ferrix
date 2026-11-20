import { Link } from '@tanstack/react-router'
import { Breadcrumb, BreadcrumbList, BreadcrumbItem, BreadcrumbLink, BreadcrumbPage, BreadcrumbSeparator } from '@/components/ui/breadcrumb'

export function ProjectBreadcrumb({ projectId, projectName, environmentId, environmentName, serviceName }: { projectId: string; projectName?: string; environmentId?: string; environmentName?: string; serviceName?: string }) {
  return <Breadcrumb aria-label="Project navigation"><BreadcrumbList className="break-words">
    <BreadcrumbItem><BreadcrumbLink asChild><Link to="/projects">Projects</Link></BreadcrumbLink></BreadcrumbItem><BreadcrumbSeparator />
    <BreadcrumbItem><BreadcrumbLink asChild><Link to="/projects/$projectId" params={{ projectId }}>{projectName ?? 'Project'}</Link></BreadcrumbLink></BreadcrumbItem>
    {environmentName && <><BreadcrumbSeparator /><BreadcrumbItem>{serviceName ? <BreadcrumbLink asChild><a href={`/projects/${projectId}?environment=${environmentId}`}>{environmentName}</a></BreadcrumbLink> : <BreadcrumbPage>{environmentName}</BreadcrumbPage>}</BreadcrumbItem></>}
    {serviceName && <><BreadcrumbSeparator /><BreadcrumbItem><BreadcrumbPage>{serviceName}</BreadcrumbPage></BreadcrumbItem></>}
  </BreadcrumbList></Breadcrumb>
}
