import { Link } from '@tanstack/react-router'
import { ArrowRight, Box, Database, Layers3, Globe } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Card, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'

const features = [
  { icon: Box, title: 'Applications', description: 'Deploy Docker images, follow deployments and inspect container logs.' },
  { icon: Database, title: 'Databases', description: 'Run databases alongside your applications in organized project environments.' },
  { icon: Globe, title: 'Websites', description: 'Serve static websites through an Application running Nginx or Caddy, with an automatic HTTPS address.' },
]

export function LandingPage() {
  return <div className="flex min-h-svh flex-col bg-background text-foreground">
    <a href="#main" className="sr-only focus:not-sr-only focus:absolute focus:bg-background focus:p-4">Skip to content</a>
    <header className="border-b">
      <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-6 py-5 lg:px-8">
        <Link to="/" className="flex items-center gap-2.5 text-lg font-semibold tracking-tight" aria-label="Ferrix home"><Layers3 className="size-6" aria-hidden="true" />Ferrix</Link>
        <nav aria-label="Main navigation" className="flex items-center gap-2"><Button asChild variant="ghost"><Link to="/login">Sign in</Link></Button><Button asChild><Link to="/signup">Get started<ArrowRight data-icon="inline-end" /></Link></Button></nav>
      </div>
    </header>
    <main id="main" className="mx-auto flex w-full max-w-6xl flex-1 flex-col justify-center gap-12 px-6 py-16 sm:py-24 lg:px-8" tabIndex={-1}>
      <section className="flex flex-col items-center gap-6 text-center" aria-labelledby="landing-title">
        <Badge variant="outline"><Box data-icon="inline-start" />Self-hosted deployment management</Badge>
        <h1 id="landing-title" className="max-w-3xl text-4xl font-semibold leading-tight tracking-tight sm:text-5xl lg:text-6xl">Your containers.<br />One clear workspace.</h1>
        <p className="max-w-xl text-base leading-relaxed text-muted-foreground sm:text-lg">Deploy applications and databases on your own server. Keep projects, environments and container operations together with Ferrix.</p>
        <div className="flex flex-wrap justify-center gap-3"><Button asChild><Link to="/signup">Get started<ArrowRight data-icon="inline-end" /></Link></Button><Button asChild variant="outline"><Link to="/login">Sign in to Ferrix</Link></Button></div>
        <p className="text-sm text-muted-foreground">Set up your administrator account, or sign in with your team account.</p>
      </section>
      <section aria-label="Deployment tools" className="grid gap-4 md:grid-cols-3">{features.map(({ icon: Icon, title, description }) => <Card key={title}><CardHeader><Icon className="mb-2 size-5 text-muted-foreground" aria-hidden="true" /><CardTitle><h2>{title}</h2></CardTitle><CardDescription>{description}</CardDescription></CardHeader></Card>)}</section>
    </main>
    <footer className="border-t"><div className="mx-auto flex max-w-6xl flex-col justify-between gap-2 px-6 py-5 text-xs text-muted-foreground sm:flex-row lg:px-8"><span>Ferrix · Deploy with confidence</span><p>Your infrastructure. Your control.</p></div></footer>
  </div>
}
