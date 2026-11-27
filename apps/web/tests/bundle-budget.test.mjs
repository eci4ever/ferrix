import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, statSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { gzipSync } from 'node:zlib'

const dist = fileURLToPath(new URL('../dist/', import.meta.url))
const manifest = JSON.parse(readFileSync(`${dist}.vite/manifest.json`, 'utf8'))

test('production entry and its eager JavaScript stay within the loading budget', () => {
  const eager = new Set()
  function visit(key) {
    if (eager.has(key)) return
    eager.add(key)
    for (const dependency of manifest[key].imports ?? []) visit(dependency)
  }
  visit('index.html')
  const files = [...eager].map(key => manifest[key].file)
  const bytes = files.reduce((sum, file) => sum + statSync(dist + file).size, 0)
  const gzip = files.reduce((sum, file) => sum + gzipSync(readFileSync(dist + file)).length, 0)
  console.log(`Initial JavaScript: ${bytes} bytes; ${gzip} gzip bytes across ${files.length} files`)
  assert.ok(statSync(dist + manifest['index.html'].file).size < 200_000, 'Entry exceeds 200 kB; defer page-specific code')
  assert.ok(bytes < 850_000, 'Eager JavaScript exceeds 850 kB; check static dependencies')
  for (const chunk of Object.values(manifest)) {
    if (chunk.file.endsWith('.js')) assert.ok(statSync(dist + chunk.file).size < 500_000, `${chunk.file} exceeds 500 kB`)
  }
})
