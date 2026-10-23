import { createFileRoute } from '@tanstack/react-router'
import { browserProxy } from '../lib/browser-proxy.server'

export const Route = createFileRoute('/api/$')({
  server: {
    handlers: {
      GET: ({ request }) => browserProxy(request),
      POST: ({ request }) => browserProxy(request),
      PUT: ({ request }) => browserProxy(request),
      PATCH: ({ request }) => browserProxy(request),
      DELETE: ({ request }) => browserProxy(request),
      HEAD: ({ request }) => browserProxy(request),
      OPTIONS: ({ request }) => browserProxy(request),
    },
  },
})
