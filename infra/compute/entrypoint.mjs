import { spawn } from 'node:child_process'
import { pathToFileURL } from 'node:url'

// Commands are fixed in the actual entrypoint, not environment-controlled.
// Parameter injection exists only for isolated process-lifecycle unit tests.
export async function supervise({
  api = ['twitter-clone-api'],
  web = [process.execPath, '/app/.output/server/index.mjs'],
  graceMs = 10_000,
} = {}) {
  const active = new Set()
  let closing = false
  let result = 1
  let timer
  function stop(code) {
    if (closing) return
    closing = true
    result = code
    for (const child of active) child.kill('SIGTERM')
    timer = setTimeout(() => {
      for (const child of active) child.kill('SIGKILL')
    }, graceMs)
  }
  const term = () => stop(143)
  const interrupt = () => stop(130)
  process.on('SIGTERM', term)
  process.on('SIGINT', interrupt)
  function start(command, extra = []) {
    const child = spawn(command[0], [...command.slice(1), ...extra], {
      stdio: 'inherit', env: process.env,
    })
    active.add(child)
    return new Promise((resolve) => {
      let settled = false
      function done(code) {
        if (settled) return
        settled = true
        active.delete(child)
        resolve(code)
      }
      child.once('error', () => done(1))
      child.once('exit', (code) => done(code ?? 1))
      if (closing) child.kill('SIGTERM')
    })
  }
  try {
    const migration = await start(api, ['migrate'])
    if (closing) return result
    if (migration !== 0) return 1
    const rust = start(api)
    const frontend = start(web)
    // Even an unexpected successful child exit means half an app is gone.
    await Promise.race([rust, frontend])
    stop(1)
    await Promise.all([rust, frontend])
    return result
  } finally {
    clearTimeout(timer)
    process.removeListener('SIGTERM', term)
    process.removeListener('SIGINT', interrupt)
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.env.APP_ENV !== 'production' || process.env.WEB_RUNTIME_MODE !== 'compute') {
    throw new Error('Combined runtime requires production Compute mode')
  }
  process.exitCode = await supervise()
}
