import assert from 'node:assert/strict'
import { test } from 'node:test'
import { codePoints, validPostBody } from '../src/lib/text.ts'

test('composer counts Unicode code points and uses Rust-compatible Unicode whitespace', () => {
  assert.equal(codePoints('😀'), 1)
  assert.equal(codePoints('e\u0301'), 2)
  assert(validPostBody('😀'.repeat(280)))
  assert(!validPostBody('😀'.repeat(281)))
  for (const body of ['', ' ', '\t\n', '\u0085\u2003']) assert(!validPostBody(body))
  assert(validPostBody('\ufeff'))
  assert(validPostBody('<script>plain text</script>'))
})
