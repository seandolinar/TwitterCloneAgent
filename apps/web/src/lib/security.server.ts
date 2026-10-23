import '@tanstack/react-start/server-only'
import { randomBytes } from 'node:crypto'

// A WeakMap holds only a random per-response nonce, never identity or query data.
const nonces = new WeakMap<Request, string>()
export function nonceFor(request: Request): string {
  let nonce = nonces.get(request)
  if (!nonce) { nonce = randomBytes(32).toString('base64'); nonces.set(request, nonce) }
  return nonce
}

export function securityHeaders(nonce: string, production = process.env.NODE_ENV === 'production'): Headers {
  const script = production ? `'self' 'nonce-${nonce}' 'strict-dynamic'` : "'self' 'unsafe-inline' 'unsafe-eval'"
  const style = production ? "'self'" : "'self' 'unsafe-inline'"
  return new Headers({
    'Cache-Control': 'private, no-store',
    'Referrer-Policy': 'no-referrer',
    'X-Content-Type-Options': 'nosniff',
    'X-Frame-Options': 'DENY',
    'Permissions-Policy': 'camera=(), microphone=(), geolocation=()',
    'Content-Security-Policy': [
      "default-src 'none'", `script-src ${script}`, `style-src ${style}`,
      "img-src 'self' data:", "font-src 'self'", production ? "connect-src 'self'" : "connect-src 'self' ws: wss:",
      "base-uri 'none'", "form-action 'self'", "frame-ancestors 'none'", "object-src 'none'",
    ].join('; '),
  })
}
