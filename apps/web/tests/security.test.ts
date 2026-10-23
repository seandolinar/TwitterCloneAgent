import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { PlainText } from '../src/components/plain-text.ts'
import { nonceFor, securityHeaders } from '../src/lib/security.server.ts'

test('plain-text component escapes hostile markup rather than producing executable elements', () => {
  const input = `<script>window.stolen=true</script><img src=x onerror="alert('fixture')">&`
  const html = renderToStaticMarkup(createElement(PlainText, { text: input }))
  assert(!html.includes('<script>'))
  assert(!html.includes('<img'))
  assert(html.includes('&lt;script&gt;'))
  assert(html.includes('&amp;'))
})

test('nonces are stable only within one request and production policy forbids unnonced inline code', () => {
  const first = new Request('http://web/')
  const second = new Request('http://web/')
  assert.equal(nonceFor(first), nonceFor(first))
  assert.notEqual(nonceFor(first), nonceFor(second))
  const headers = securityHeaders(nonceFor(first), true)
  const policy = headers.get('Content-Security-Policy')!
  assert(policy.includes(`'nonce-${nonceFor(first)}'`))
  assert(!policy.includes("'unsafe-inline'"))
  assert(!policy.includes("'unsafe-eval'"))
  assert(policy.includes("frame-ancestors 'none'"))
  assert(policy.includes("form-action 'self'"))
  assert.equal(headers.get('X-Frame-Options'), 'DENY')
  assert(headers.get('Cache-Control')?.includes('no-store'))
})
