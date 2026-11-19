import { test } from 'node:test'
import assert from 'node:assert/strict'
import { request } from '../src/lib/api.ts'

test('accepts empty 202 replies and preserves JSON responses and failures', async () => {
  const original = globalThis.fetch
  try {
    globalThis.fetch = async () => new Response(null, { status: 202 })
    assert.equal(await request('/auth/student-register'), undefined)
    assert.equal(await request('/student-hosting/test/approve'), undefined)
    globalThis.fetch = async () => Response.json({ status: 'queued' }, { status: 202 })
    assert.deepEqual(await request('/deploy'), { status: 'queued' })
    globalThis.fetch = async () => Response.json({ error: 'Registration is closed.' }, { status: 403 })
    await assert.rejects(request('/auth/student-register'), /Registration is closed/)
  } finally { globalThis.fetch = original }
})
