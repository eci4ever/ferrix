import { test } from 'node:test'
import assert from 'node:assert/strict'
import { request } from '../src/lib/api.ts'

test('accepts empty 202 replies and preserves JSON responses and failures', async () => {
  const original = globalThis.fetch
  try {
    globalThis.fetch = async () => new Response(null, { status: 202 })
    assert.equal(await request('/deploy'), undefined)
    assert.equal(await request('/applications/test/deploy'), undefined)
    globalThis.fetch = async () => Response.json({ status: 'queued' }, { status: 202 })
    assert.deepEqual(await request('/deploy'), { status: 'queued' })
    globalThis.fetch = async () => Response.json({ error: 'Deployment is unavailable.' }, { status: 403 })
    await assert.rejects(request('/deploy'), /Deployment is unavailable/)
  } finally { globalThis.fetch = original }
})
