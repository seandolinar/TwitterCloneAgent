import assert from 'node:assert/strict'
import { test } from 'node:test'
import { clientIP, privateOrigin, runtimeMode } from '../src/lib/runtime.server.ts'

test('Compute trust is explicit, loopback-only, and rejects ambiguous forwarding', () => {
  const previous = process.env.WEB_RUNTIME_MODE
  try {
    process.env.WEB_RUNTIME_MODE = 'browser'
    const request = new Request('http://fixture.test', { headers: {
      'X-Forwarded-For': '203.0.113.1', 'X-Forwarded-Proto': 'https',
    } })
    assert.equal(clientIP(request, '192.0.2.1'), '192.0.2.1')
    process.env.WEB_RUNTIME_MODE = 'compute'
    assert.equal(runtimeMode(), 'compute')
    assert.equal(clientIP(request, '192.0.2.1'), '203.0.113.1')
    assert.equal(privateOrigin(new URL('http://127.0.0.1:8081')).hostname, '127.0.0.1')
    for (const origin of ['https://example.com', 'http://example.com', 'http://0.0.0.0:8081']) {
      assert.throws(() => privateOrigin(new URL(origin)))
    }
    for (const headers of [
      { 'X-Forwarded-For': '203.0.113.1, 203.0.113.2', 'X-Forwarded-Proto': 'https' },
      { 'X-Forwarded-For': '203.0.113.1', 'X-Forwarded-Proto': 'https, https' },
    ]) {
      assert.throws(() => clientIP(new Request('http://fixture.test', { headers }), '192.0.2.1'))
    }
    process.env.WEB_RUNTIME_MODE = 'infer-from-headers'
    assert.throws(() => runtimeMode())
  } finally {
    if (previous === undefined) delete process.env.WEB_RUNTIME_MODE
    else process.env.WEB_RUNTIME_MODE = previous
  }
})
