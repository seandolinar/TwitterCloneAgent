import '@tanstack/react-start/server-only'
import { isIP } from 'node:net'

export function runtimeMode(): 'browser' | 'compute' {
  const mode = process.env.WEB_RUNTIME_MODE ?? 'browser'
  if (mode !== 'browser' && mode !== 'compute') throw new Error('Invalid WEB_RUNTIME_MODE')
  return mode
}

export function privateOrigin(origin: URL): URL {
  if (runtimeMode() === 'compute' && (origin.protocol !== 'http:' ||
    !['127.0.0.1', '[::1]'].includes(origin.hostname))) {
    throw new Error('Compute upstreams must be loopback HTTP origins')
  }
  return origin
}

export function clientIP(request: Request, peer: string): string {
  if (runtimeMode() === 'browser') return peer
  // Only valid behind the exclusively managed Compute ingress. Presence of a
  // header never turns browser mode into gateway trust. Duplicates/chains fail.
  const ip = request.headers.get('x-forwarded-for')
  if (!ip || !isIP(ip) || request.headers.get('x-forwarded-proto') !== 'https') {
    throw new Error('Missing trusted Compute forwarding context')
  }
  return ip
}
