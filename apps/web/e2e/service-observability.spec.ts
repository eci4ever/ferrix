import { test, expect } from '@playwright/test'

test('Service logs search and follow safely, and monitoring compares usage with limits', async ({
  page,
}) => {
  await page.clock.install()
  let logRequests = 0
  let logText = 'INFO ready\nERROR <img src=x onerror="alert(1)">\n'
  let container = 'active-container'
  let logDenied = false
  let metricsError = 0
  let stopped = false
  let truncated = false
  const tails: string[] = []
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url())
    const path = url.pathname
    const now = new Date().toISOString()
    if (path === '/api/v1/services/service/logs') {
      logRequests++
      tails.push(url.searchParams.get('tail') ?? '')
      await route.fulfill(
        logDenied
          ? { status: 401, json: { error: 'Session expired.' } }
          : {
              json: {
                logs: logText,
                container_id: container,
                checked_at: now,
                truncated,
              },
            },
      )
      return
    }
    if(path==='/api/v1/services/service/history'){await route.fulfill({json:{samples:[{id:1,collected_at:now,container_id:'old-container',deployment_id:'previous-deploy',image_id:'sha256:old',marker:true,error:false,data:{cpu_percent_of_limit:25,memory_percent_of_limit:50,state:'running',restart_count:0,health:'healthy'}},{id:2,collected_at:now,container_id:'active-container',deployment_id:'current-deploy',image_id:'sha256:new',marker:true,error:false,data:{cpu_percent_of_limit:30,memory_percent_of_limit:60,state:'running',restart_count:1,health:'healthy'}}],sample_seconds:30,retention_days:3,downsample_after_minutes:360,downsample_seconds:300,truncated:false,gaps_note:'Gaps are unknown.',scope:'service'}});return}
    if(path==='/api/v1/services/service/logs/history'){expect(url.searchParams.get('from')).toBeTruthy();await route.fulfill({json:{lines:[{id:1,deployment_id:'previous-deploy',container_id:'old-container',emitted_at:now,collected_at:now,message:'retained <script>bad</script> log',truncated:false}],next_before:null,collection_note:'Docker log time differs from collection time.'}});return}
    if (path === '/api/v1/services/service/metrics') {
      await route.fulfill(
        metricsError
          ? { status: metricsError, json: { error: 'Metrics unavailable.' } }
          : {
              json: {
                container_id: container,
                state: stopped ? 'exited' : 'running',
                checked_at: now,
                cpu_cores_used: stopped ? null : 0.25,
                cpu_limit_cores: 1,
                cpu_percent_of_limit: stopped ? null : 25,
                memory_used_bytes: stopped ? null : 256 * 1024 * 1024,
                memory_raw_bytes: stopped ? null : 320 * 1024 * 1024,
                memory_limit_bytes: 512 * 1024 * 1024,
                memory_percent_of_limit: stopped ? null : 50,
              },
            },
      )
      return
    }
    const responses: Record<string, unknown> = {
      '/api/v1/auth/session': {
        name: 'Member',
        role: 'member',
        system_role: 'user',
        organization_id: 'org',
        organization_name: 'Team',
      },
      '/api/v1/auth/setup': { setup_required: false },
      '/api/v1/auth/organizations': [],
      '/api/v1/status': { database: true, docker: true, version: 'test' },
      '/api/v1/server-time': {
        unix_ms: Date.now(),
        timezone: 'UTC',
        utc_offset: '+00:00',
      },
      '/api/v1/projects/project/environments': {
        project_name: 'Portal',
        environments: [{ id: 'production', name: 'production' }],
      },
      '/api/v1/services/service': {
        id: 'service',
        project_id: 'project',
        environment_id: 'production',
        kind: 'application',
        name: 'Web app',
        image: 'nginx:alpine',
        active_image: 'nginx:alpine',
        active_deployment_id: 'release',
        deployed: true,
        created_at: now,
      },
      '/api/v1/services/service/runtime': {
        state: 'running',
        container_id: container,
      },
      '/api/v1/services/service/deployments': [],
    }
    await route.fulfill({ json: responses[path] ?? {} })
  })
  await page.goto('/projects/project/services/service')
  await page.getByRole('tab', { name: 'Logs', exact: true }).click()
  const output = page.getByRole('region', {
    name: 'Container log output',
    exact: true,
  })
  await expect(output).toContainText('INFO ready')
  expect(tails).toContain('500')
  await page.getByLabel('Search logs', { exact: true }).fill('error')
  await expect(output).not.toContainText('INFO ready')
  await expect(output).toContainText('<img src=x')
  await expect(output.locator('img')).toHaveCount(0)
  await page.getByRole('button', { name: 'Clear search', exact: true }).click()
  logText = 'INFO ready\nINFO first live update\n'
  await page
    .getByRole('switch', { name: 'Live tail', exact: true })
    .click()
  await expect(output).toContainText('first live update')
  logText = 'INFO second live update\n'
  await page.clock.fastForward(2100)
  await expect(output).toContainText('second live update')
  await expect(output).not.toContainText('first live update')
  await page
    .getByRole('switch', { name: 'Live tail', exact: true })
    .click()
  const pausedRequests = logRequests
  await page.clock.fastForward(6000)
  expect(logRequests).toBe(pausedRequests)
  await page.getByLabel('Log lines', { exact: true }).selectOption('1000')
  await expect.poll(() => tails.at(-1)).toBe('1000')
  container = 'new-container'
  logText = 'INFO new deployment ready\n'
  truncated = true
  await page.getByRole('button', { name: 'Refresh logs', exact: true }).click()
  await expect(output).toContainText('new deployment ready')
  await expect(
    page.getByText('Container changed. Showing the active deployment’s logs.', {
      exact: true,
    }),
  ).toBeVisible()
  await expect(
    page.getByText(
      'Output exceeds the buffer limit. Showing the newest 64 KiB.',
      { exact: true },
    ),
  ).toBeVisible()
  await page.setViewportSize({ width: 390, height: 844 })
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true)
  await page.screenshot({
    path: '/private/tmp/ferrix-service-logs-mobile.png',
    fullPage: true,
    animations: 'disabled',
  })
  logDenied = true
  await page
    .getByRole('switch', { name: 'Live tail', exact: true })
    .click()
  await expect(output).toHaveText('Logs unavailable.')
  await expect(
    page.getByRole('switch', { name: 'Live tail', exact: true }),
  ).not.toBeChecked()
  await page.getByRole('tab', { name: 'Monitoring', exact: true }).click()
  await expect(
    page.getByRole('meter', { name: 'CPU limit usage', exact: true }),
  ).toHaveAttribute('aria-valuenow', '25')
  await expect(
    page.getByRole('meter', { name: 'RAM limit usage', exact: true }),
  ).toHaveAttribute('aria-valuenow', '50')
  await expect(
    page.getByText('256 MiB used · 512 MiB limit', { exact: true }),
  ).toBeVisible()
  await page.screenshot({
    path: '/private/tmp/ferrix-service-metrics-mobile.png',
    fullPage: true,
    animations: 'disabled',
  })
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.screenshot({
    path: '/private/tmp/ferrix-service-metrics-desktop.png',
    fullPage: true,
    animations: 'disabled',
  })
  stopped = true
  await page.clock.fastForward(5100)
  await expect(
    page.getByRole('meter', { name: 'CPU limit usage', exact: true }),
  ).toHaveAttribute('aria-valuetext', 'Unavailable')
  await expect(
    page.getByRole('meter', { name: 'RAM limit usage', exact: true }),
  ).toHaveAttribute('aria-valuetext', 'Unavailable')
  metricsError = 503
  await page.clock.fastForward(5100)
  await expect(
    page.getByText('Displayed readings are from the last successful update.', {
      exact: true,
    }),
  ).toBeVisible()
  await page.getByRole('button',{name:'View history',exact:true}).click()
  await expect(page.getByRole('img',{name:/Retained percent history/})).toBeVisible()
  await expect(page.getByText('previous-deploy',{exact:true})).toBeVisible()
  await expect(page.getByText('retained <script>bad</script> log',{exact:true})).toBeVisible()
  await page.getByLabel('Search retained logs',{exact:true}).fill('retained')
  await page.getByRole('button',{name:'Search logs',exact:true}).click()
  await expect(page.locator('script').filter({hasText:'bad'})).toHaveCount(0)
  await page.setViewportSize({width:390,height:844})
  await expect.poll(()=>page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true)
  metricsError = 401
  await page.clock.fastForward(5100)
  await expect(page.getByRole('meter')).toHaveCount(0)
})
