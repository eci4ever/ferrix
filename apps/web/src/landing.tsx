import { Link } from '@tanstack/react-router'
import { ArrowRight, Box, Check, Database, FileCode2, Globe2, Layers3, Package, ShieldCheck } from 'lucide-react'
import { Button, buttonVariants } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'

const dockerFeatures = ['Deploy Docker images and Compose stacks', 'Organize apps, databases and environments', 'Follow deployments and read container logs']
const hostingFeatures = ['Upload your PHP project folder', 'Get your own MariaDB database and user', 'Open your website with an HTTPS address']

export function LandingPage() {
  return <div className="flex min-h-svh flex-col bg-background text-foreground">
    <a href="#main" className="sr-only focus:not-sr-only focus:absolute focus:z-10 focus:bg-background focus:p-4">Skip to content</a>
    <header className="border-b">
      <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-6 py-5 lg:px-8">
        <Link to="/" className="flex items-center gap-2.5 text-lg font-semibold tracking-tight" aria-label="Ferrix home"><Layers3 className="size-6" aria-hidden="true" />Ferrix</Link>
        <nav aria-label="Main navigation" className="flex items-center gap-3">
          <Button asChild variant="ghost" className="hidden sm:inline-flex"><a href="#workspaces">Explore workspaces</a></Button>
          <Button asChild variant="outline"><Link to="/login">Sign in<ArrowRight data-icon="inline-end" /></Link></Button>
        </nav>
      </div>
    </header>

    <main id="main" className="mx-auto flex w-full max-w-6xl flex-1 flex-col gap-10 px-6 py-12 sm:gap-12 sm:py-16 lg:px-8" tabIndex={-1}>
      <section className="flex flex-col items-center gap-5 text-center" aria-labelledby="landing-title">
        <Badge variant="outline"><Layers3 data-icon="inline-start" />Your infrastructure. Your workspace.</Badge>
        <h1 id="landing-title" className="max-w-3xl text-4xl font-semibold leading-tight tracking-tight sm:text-5xl lg:text-6xl">Choose how your<br className="hidden sm:block" /> project goes live.</h1>
        <p className="max-w-xl text-base leading-relaxed text-muted-foreground sm:text-lg">Deploy containers or host your PHP website.<br className="hidden sm:block" /> Two straightforward ways to bring your project online with Ferrix.</p>
      </section>

      <section id="workspaces" aria-label="Choose your workspace" className="grid scroll-mt-6 gap-6 md:grid-cols-2">
        <Link to="/login" aria-labelledby="docker-title" aria-describedby="docker-description" className="group block rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-4 focus-visible:ring-offset-background">
          <Card className="h-full transition-shadow group-hover:ring-foreground/25 motion-reduce:transition-none">
            <CardHeader>
              <div className="mb-4 flex items-center justify-between gap-4"><span className="flex size-12 items-center justify-center rounded-xl border bg-background"><Box className="size-6" aria-hidden="true" /></span><Badge variant="secondary">For developers & teams</Badge></div>
              <CardTitle><h2 id="docker-title">Docker / Sandbox</h2></CardTitle>
              <CardDescription id="docker-description">A workspace for container-based projects. Bring your Docker image or Compose file and manage your deployment.</CardDescription>
            </CardHeader>
            <CardContent className="flex flex-1 flex-col gap-6">
              <div aria-hidden="true" className="flex items-center justify-between gap-2 rounded-lg border bg-background px-4 py-5 text-muted-foreground">
                <div className="flex flex-col items-center gap-2"><Package className="size-5" /><span className="text-xs">Docker image</span></div><ArrowRight className="size-4 shrink-0" /><div className="flex flex-col items-center gap-2"><Box className="size-5" /><span className="text-xs">Container</span></div><ArrowRight className="size-4 shrink-0" /><div className="flex flex-col items-center gap-2"><Globe2 className="size-5" /><span className="text-xs">Your app</span></div>
              </div>
              <ul className="flex flex-col gap-3">{dockerFeatures.map(feature => <li key={feature} className="flex items-start gap-2.5 text-sm"><Check className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" /><span>{feature}</span></li>)}</ul>
            </CardContent>
            <CardFooter className="flex flex-col items-stretch gap-3">
              <span className={buttonVariants({ variant: 'outline' })}>Open developer workspace<ArrowRight data-icon="inline-end" /></span>
              <p className="text-center text-xs leading-relaxed text-muted-foreground">Sign in with your administrator or team invitation account.</p>
            </CardFooter>
          </Card>
        </Link>

        <Link to="/student-register" aria-labelledby="hosting-title" aria-describedby="hosting-description" className="group block rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-4 focus-visible:ring-offset-background">
          <Card className="h-full transition-shadow group-hover:ring-foreground/25 motion-reduce:transition-none">
            <CardHeader>
              <div className="mb-4 flex items-center justify-between gap-4"><span className="flex size-12 items-center justify-center rounded-xl border bg-background"><FileCode2 className="size-6" aria-hidden="true" /></span><Badge variant="secondary">For PHP projects & students</Badge></div>
              <CardTitle><h2 id="hosting-title">Shared Hosting</h2></CardTitle>
              <CardDescription id="hosting-description">A home for your PHP website, with a private database on a shared MariaDB server. Upload your files and start building.</CardDescription>
            </CardHeader>
            <CardContent className="flex flex-1 flex-col gap-6">
              <div aria-hidden="true" className="flex items-center justify-between gap-2 rounded-lg border bg-background px-4 py-5 text-muted-foreground">
                <div className="flex flex-col items-center gap-2"><FileCode2 className="size-5" /><span className="text-xs">PHP files</span></div><span className="text-sm">+</span><div className="flex flex-col items-center gap-2"><Database className="size-5" /><span className="text-xs">MariaDB</span></div><ArrowRight className="size-4 shrink-0" /><div className="flex flex-col items-center gap-2"><Globe2 className="size-5" /><span className="text-xs">Your website</span></div>
              </div>
              <ul className="flex flex-col gap-3">{hostingFeatures.map(feature => <li key={feature} className="flex items-start gap-2.5 text-sm"><Check className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" /><span>{feature}</span></li>)}</ul>
            </CardContent>
            <CardFooter className="flex flex-col items-stretch gap-3">
              <span className={buttonVariants()}>Request shared hosting<ArrowRight data-icon="inline-end" /></span>
              <p className="text-center text-xs leading-relaxed text-muted-foreground">Register first. Your administrator approves hosting access.</p>
            </CardFooter>
          </Card>
        </Link>
      </section>

      <div className="flex flex-col items-center justify-center gap-3 text-center text-sm text-muted-foreground sm:flex-row sm:gap-5">
        <span className="flex items-center gap-2"><ShieldCheck className="size-4" aria-hidden="true" />A separate workspace for your projects</span>
        <Separator orientation="vertical" className="hidden h-4 sm:block" />
        <p>Already have an account? <Link to="/login" className="font-medium text-foreground underline underline-offset-4">Sign in to Ferrix</Link></p>
      </div>
    </main>

    <footer className="border-t"><div className="mx-auto flex max-w-6xl flex-col items-start justify-between gap-2 px-6 py-5 text-xs text-muted-foreground sm:flex-row sm:items-center lg:px-8"><span>Ferrix · Deploy & host</span><p>Need access? Contact your administrator.</p></div></footer>
  </div>
}
