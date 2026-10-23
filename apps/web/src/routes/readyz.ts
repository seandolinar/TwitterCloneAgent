import { createFileRoute } from '@tanstack/react-router'
import { backendOrigin } from '../lib/browser-proxy.server'
import { privateOrigin } from '../lib/runtime.server'

export const Route = createFileRoute('/readyz')({
  server: { handlers: { GET: async () => {
    let ready = false
    try {
      const origin = privateOrigin(backendOrigin())
      const response = await fetch(new URL('/readyz', origin), { redirect: 'manual',
        signal: AbortSignal.timeout(2000), cache: 'no-store' })
      ready = response.status === 200
      await response.body?.cancel()
    } catch { /* no configuration or dependency details in health responses */ }
    return new Response(null, { status: ready ? 200 : 503, headers: { 'Cache-Control': 'no-store' } })
  } } },
})
