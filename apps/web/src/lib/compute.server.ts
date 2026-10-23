import '@tanstack/react-start/server-only'
import { backendOrigin, boundedBody, canonicalTarget, nodePeer, privateHeaders, proxyError, requestPolicy } from './browser-proxy.server'
import { clientIP, privateOrigin, runtimeMode } from './runtime.server'

export async function integrationProxy(request: Request): Promise<Response> {
  if (runtimeMode() !== 'compute') return proxyError(404, 'not_found', 'The resource was not found.')
  const peer = nodePeer(request)
  if (!peer || !canonicalTarget(peer.target)) return proxyError(400, 'invalid_path', 'The request path is not allowed.')
  const url = new URL(request.url)
  const checked = new URL(url)
  checked.pathname = '/api' + checked.pathname
  const rejection = requestPolicy(request.method, checked)
  if (rejection) return rejection
  const principal = request.headers.get('x-unkey-principal')
  if (!principal || principal.length > 16384) {
    return proxyError(401, 'invalid_key', 'A valid integration key is required.')
  }
  let destination: URL
  let ip: string
  try {
    destination = new URL(url.pathname + url.search,
      privateOrigin(backendOrigin(process.env.GATEWAY_API_BASE_URL ?? 'http://127.0.0.1:8082')))
    ip = clientIP(request, peer.ip)
  } catch { return proxyError(503, 'service_unavailable', 'The service is temporarily unavailable.') }
  // Preserve only the managed gateway principal on this branch. Raw Bearer,
  // cookies, CSRF, forwarded fields, and caller-private headers never reach Rust.
  const headers = new Headers({ 'X-Unkey-Principal': principal, 'X-Twitterclone-Client-IP': ip })
  for (const name of ['content-type', 'accept']) {
    const value = request.headers.get(name)
    if (value !== null) headers.set(name, value)
  }
  try {
    const signal = AbortSignal.any([request.signal, AbortSignal.timeout(8000)])
    const body = request.method === 'GET' ? undefined : await boundedBody(request, signal)
    const upstream = await fetch(destination, { method: request.method, headers, body, signal, redirect: 'manual' })
    if (upstream.status >= 300 && upstream.status < 400) {
      await upstream.body?.cancel()
      return proxyError(503, 'service_unavailable', 'The service is temporarily unavailable.')
    }
    const responseHeaders = privateHeaders()
    for (const name of ['content-type', 'retry-after', 'allow', 'x-request-id']) {
      const value = upstream.headers.get(name)
      if (value !== null) responseHeaders.set(name, value)
    }
    return new Response(upstream.body, { status: upstream.status, headers: responseHeaders })
  } catch (error) {
    return error instanceof RangeError
      ? proxyError(413, 'request_too_large', 'The request body exceeds the allowed size.')
      : proxyError(503, 'service_unavailable', 'The service is temporarily unavailable.')
  }
}
