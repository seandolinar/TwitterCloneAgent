import handler, { createServerEntry } from '@tanstack/react-start/server-entry'
import { nonceFor, securityHeaders } from './lib/security.server'

export default createServerEntry({
  async fetch(request, options) {
    const nonce = nonceFor(request)
    const response = await handler.fetch(request, options)
    const headers = new Headers(response.headers)
    securityHeaders(nonce).forEach((value, name) => headers.set(name, value))
    return new Response(response.body, {
      status: response.status, statusText: response.statusText, headers,
    })
  },
})
