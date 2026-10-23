import { defineEventHandler } from 'nitro/h3'
import { canonicalTarget, nodePeer, proxyError } from '../../src/lib/browser-proxy.server'
import { integrationProxy } from '../../src/lib/compute.server'
import { runtimeMode } from '../../src/lib/runtime.server'

export default defineEventHandler((event) => {
  const peer = nodePeer(event.req)
  if (!peer) return proxyError(503, 'client_context_unavailable', 'Trusted client context is unavailable.')
  if (!canonicalTarget(peer.target)) return proxyError(400, 'invalid_path', 'The request path is not allowed.')
  const path = peer.target.split('?', 1)[0]
  if (path === '/v1' || path.startsWith('/v1/')) {
    try {
      if (runtimeMode() === 'compute') return integrationProxy(event.req)
    } catch { return proxyError(503, 'service_unavailable', 'The service is temporarily unavailable.') }
    return proxyError(404, 'not_found', 'The resource was not found.')
  }
})
