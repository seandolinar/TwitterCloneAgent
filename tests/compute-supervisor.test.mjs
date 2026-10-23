import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { setTimeout as delay } from 'node:timers/promises'

const supervisor = new URL('../infra/compute/entrypoint.mjs', import.meta.url).href
const fixture = `
const role = process.argv[1];
const migrating = process.argv.includes('migrate');
const phase = migrating ? 'migration' : role;
console.log(phase + ':start');
process.on('SIGTERM', () => { console.log(phase + ':term'); process.exit(0) });
if (migrating && process.env.FIXTURE_HOLD_MIGRATION !== '1') {
  setTimeout(() => process.exit(Number(process.env.FIXTURE_MIGRATION_STATUS || 0)), 20);
} else if (role === process.env.FIXTURE_FAIL_CHILD) {
  setTimeout(() => process.exit(7), 30);
} else { setInterval(() => {}, 1000); }
`
function run(t, env = {}) {
  const args = { api: [process.execPath, '-e', fixture, 'api'],
    web: [process.execPath, '-e', fixture, 'web'], graceMs: 500 }
  const code = `import { supervise } from ${JSON.stringify(supervisor)}; process.exitCode = await supervise(${JSON.stringify(args)})`
  const child = spawn(process.execPath, ['--input-type=module', '-e', code], {
    env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'],
  })
  const exited = once(child, 'exit')
  let output = ''
  child.stdout.on('data', (chunk) => { output += chunk })
  child.stderr.on('data', (chunk) => { output += chunk })
  t.after(() => { if (child.exitCode === null) child.kill('SIGKILL') })
  return { child, exited, output: () => output,
    async wait(marker) {
      for (let i = 0; i < 200; i++) {
        if (output.includes(marker)) return
        assert.equal(child.exitCode, null, output)
        await delay(5)
      }
      assert.fail(`Fixture did not reach ${marker}: ${output}`)
    } }
}

test('migrations precede serving and SIGTERM reaches both children', { timeout: 5000 }, async (t) => {
  const fixture = run(t)
  await fixture.wait('web:start')
  await fixture.wait('api:start')
  assert(fixture.output().indexOf('migration:start') < fixture.output().indexOf('api:start'))
  fixture.child.kill('SIGTERM')
  const [code] = await fixture.exited
  assert.equal(code, 143)
  assert(fixture.output().includes('api:term'))
  assert(fixture.output().includes('web:term'))
})
for (const role of ['api', 'web']) {
  test(`${role} failure terminates the other process`, { timeout: 5000 }, async (t) => {
    const fixture = run(t, { FIXTURE_FAIL_CHILD: role })
    const [code] = await fixture.exited
    assert.equal(code, 1)
    assert(fixture.output().includes((role === 'api' ? 'web' : 'api') + ':term'))
  })
}
test('failed migration never starts either server', { timeout: 5000 }, async (t) => {
  const fixture = run(t, { FIXTURE_MIGRATION_STATUS: '9' })
  assert.equal((await fixture.exited)[0], 1)
  assert(!fixture.output().includes('api:start'))
  assert(!fixture.output().includes('web:start'))
})
test('shutdown during migration never starts servers', { timeout: 5000 }, async (t) => {
  const fixture = run(t, { FIXTURE_HOLD_MIGRATION: '1' })
  await fixture.wait('migration:start')
  fixture.child.kill('SIGINT')
  assert.equal((await fixture.exited)[0], 130)
  assert(fixture.output().includes('migration:term'))
  assert(!fixture.output().includes('api:start'))
})
