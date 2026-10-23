import '@tanstack/react-start/server-only'
import { randomUUID } from 'node:crypto'
import { isIP } from 'node:net'
import type { IncomingMessage } from 'node:http'
import { clientIP, privateOrigin } from './runtime.server.ts'

const MAX_BODY_BYTES = 16 * 1024
const UUID = '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}'
const HANDLE = '[A-Za-z0-9_]{3,30}'
const routes: ReadonlyArray<{ path: RegExp; methods: string[]; query?: string[] }> = [
  { path: /^\/api\/auth\/(csrf|session|settings)$/, methods: ['GET'] },
  { path: /^\/api\/auth\/(register|login|logout|verify-email|resend-verification|forgot-password|reset-password|reauthenticate)$/, methods: ['POST'] },
  { path: /^\/api\/auth\/keys$/, methods: ['GET', 'POST'] },
  { path: /^\/api\/auth\/keys\/key_[A-Za-z0-9_]+$/, methods: ['DELETE'] },
  { path: /^\/api\/v1\/me$/, methods: ['GET', 'PATCH'] },
  { path: new RegExp(`^/api/v1/profiles/${HANDLE}$`), methods: ['GET'] },
  { path: new RegExp(`^/api/v1/profiles/${HANDLE}/posts$`), methods: ['GET'], query: ['limit', 'cursor'] },
  { path: /^\/api\/v1\/feed$/, methods: ['GET'], query: ['limit', 'cursor'] },
  { path: /^\/api\/v1\/posts$/, methods: ['POST'] },
  { path: new RegExp(`^/api/v1/posts/${UUID}$`), methods: ['GET', 'DELETE'] },
  { path: new RegExp(`^/api/v1/me/following/${UUID}$`), methods: ['PUT', 'DELETE'] },
  { path: new RegExp(`^/api/v1/posts/${UUID}/like$`), methods: ['PUT', 'DELETE'] },
]

export function privateHeaders(): Headers {
  return new Headers({
    'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff',
    'Referrer-Policy': 'no-referrer',
    'X-Frame-Options': 'DENY',
    'Permissions-Policy': 'camera=(), microphone=(), geolocation=()',
    'Content-Security-Policy': "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
  })
}

export function proxyError(status: number, code: string, message: string, extra?: HeadersInit): Response {
  const headers = privateHeaders()
  new Headers(extra).forEach((value, name) => headers.set(name, value))
  const requestId = randomUUID()
  headers.set('X-Request-Id', requestId)
  return Response.json({ error: { code, message }, requestId }, { status, headers })
}

// Read the raw Node target before URL/router normalization. Handles and UUIDs
// need no path encoding; query encoding remains available for opaque cursors.
export function canonicalTarget(target: string): boolean {
  const path = target.split('?', 1)[0]
  return path.startsWith('/') && !path.includes('//') &&
    !/[%\\#\x00-\x20\x7f]/.test(path) &&
    !path.split('/').some((part) => part === '.' || part === '..')
}

export function nodePeer(request: Request): { ip: string; target: string } | undefined {
  const runtime = (request as Request & {
    runtime?: { name?: string; node?: { req?: IncomingMessage } }
  }).runtime
  if (runtime?.name !== 'node') return undefined
  const incoming = runtime.node?.req
  let ip = incoming?.socket.remoteAddress
  if (!ip || !isIP(ip) || !incoming?.url) return undefined
  if (ip.startsWith('::ffff:') && isIP(ip.slice(7)) === 4) ip = ip.slice(7)
  return { ip, target: incoming.url }
}

export function backendOrigin(value = process.env.API_BASE_URL ?? 'http://127.0.0.1:8080'): URL {
  const url = new URL(value)
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password ||
    url.pathname !== '/' || url.search || url.hash) {
    throw new Error('API_BASE_URL must be a fixed HTTP origin without credentials or a path.')
  }
  return url
}

export async function boundedBody(request: Request, signal: AbortSignal): Promise<Uint8Array<ArrayBuffer> | undefined> {
  if (!request.body) return undefined
  const reader = request.body.getReader()
  const chunks: Uint8Array[] = []
  let size = 0
  const abort = () => { void reader.cancel().catch(() => {}) }
  signal.addEventListener('abort', abort, { once: true })
  try {
    while (true) {
      signal.throwIfAborted()
      const { done, value } = await reader.read()
      signal.throwIfAborted()
      if (done) break
      size += value.byteLength
      if (size > MAX_BODY_BYTES) {
        await reader.cancel()
        throw new RangeError('Request body exceeds the allowed size.')
      }
      chunks.push(value)
    }
  } finally {
    signal.removeEventListener('abort', abort)
    reader.releaseLock()
  }
  const body = new Uint8Array(size)
  let offset = 0
  for (const chunk of chunks) { body.set(chunk, offset); offset += chunk.byteLength }
  return body
}

export function requestPolicy(method: string, url: URL): Response | undefined {
  if (!canonicalTarget(url.pathname)) {
    return proxyError(400, 'invalid_path', 'The request path is not allowed.')
  }
  const route = routes.find((route) => route.path.test(url.pathname))
  if (!route) return proxyError(404, 'not_found', 'The resource was not found.')
  if (!route.methods.includes(method)) {
    return proxyError(405, 'method_not_allowed', 'The method is not allowed.', { Allow: route.methods.join(', ') })
  }
  for (const name of url.searchParams.keys()) {
    if (!route.query?.includes(name) || url.searchParams.getAll(name).length !== 1) {
      return proxyError(422, 'invalid_query', 'Use only supported query parameters.')
    }
  }
}

export async function proxyBrowserRequest(request: Request, ip: string, backend: URL): Promise<Response> {
  const url = new URL(request.url)
  if (!isIP(ip)) return proxyError(503, 'client_context_unavailable', 'Trusted client context is unavailable.')
  const rejection = requestPolicy(request.method, url)
  if (rejection) return rejection
  const headers = new Headers()
  // Allowlisting rather than copying then stripping keeps future identity and
  // hop-by-hop headers out of this trust boundary by default.
  for (const name of ['cookie', 'origin', 'x-csrf-token', 'content-type', 'accept']) {
    const value = request.headers.get(name)
    if (value !== null) headers.set(name, value)
  }
  headers.set('X-Twitterclone-Client-IP', ip)
  const destination = new URL(url.pathname.replace(/^\/api\//, '/browser/'), backend)
  destination.search = url.search
  try {
    const signal = AbortSignal.any([request.signal, AbortSignal.timeout(8000)])
    const body = ['GET', 'HEAD'].includes(request.method) ? undefined : await boundedBody(request, signal)
    const upstream = await fetch(destination, {
      method: request.method, headers, body, redirect: 'manual',
      signal,
    })
    if (upstream.status >= 300 && upstream.status < 400) {
      await upstream.body?.cancel()
      return proxyError(503, 'service_unavailable', 'The service is temporarily unavailable.')
    }
    const responseHeaders = privateHeaders()
    for (const name of ['content-type', 'retry-after', 'allow', 'x-request-id']) {
      const value = upstream.headers.get(name)
      if (value !== null) responseHeaders.set(name, value)
    }
    for (const cookie of upstream.headers.getSetCookie()) responseHeaders.append('Set-Cookie', cookie)
    return new Response(upstream.body, { status: upstream.status, headers: responseHeaders })
  } catch (error) {
    if (error instanceof RangeError) return proxyError(413, 'request_too_large', 'The request body exceeds the allowed size.')
    return proxyError(503, 'service_unavailable', 'The service is temporarily unavailable.')
  }
}

export async function browserProxy(request: Request): Promise<Response> {
  const peer = nodePeer(request)
  if (!peer) return proxyError(503, 'client_context_unavailable', 'Trusted client context is unavailable.')
  if (!canonicalTarget(peer.target)) return proxyError(400, 'invalid_path', 'The request path is not allowed.')
  let backend: URL
  let ip: string
  try { backend = privateOrigin(backendOrigin()); ip = clientIP(request, peer.ip) } catch {
    return proxyError(503, 'service_unavailable', 'The service is temporarily unavailable.')
  }
  return proxyBrowserRequest(request, ip, backend)
}

// Instantiate only within a loader/server function, using its current request.
// The closure owns no shared query cache and cannot perform SSR mutations.
export function createServerApi(request: Request) {
  const peer = nodePeer(request)
  const headers = new Headers()
  for (const name of ['cookie', 'origin']) {
    const value = request.headers.get(name)
    if (value !== null) headers.set(name, value)
  }
  return {
    async read(path: string): Promise<Response> {
      if (!peer || !canonicalTarget(peer.target)) return proxyError(503, 'client_context_unavailable', 'Trusted client context is unavailable.')
      if (!path.startsWith('/api/') || !canonicalTarget(path)) return proxyError(400, 'invalid_path', 'The request path is not allowed.')
      let backend: URL
      let ip: string
      try { backend = privateOrigin(backendOrigin()); ip = clientIP(request, peer.ip) } catch {
        return proxyError(503, 'service_unavailable', 'The service is temporarily unavailable.')
      }
      const target = new URL(path, new URL(request.url).origin)
      return proxyBrowserRequest(new Request(target, { headers, signal: request.signal }), ip, backend)
    },
  }
}
