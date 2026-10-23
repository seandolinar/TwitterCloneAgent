import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/healthz')({
  server: {
    handlers: {
      GET: () => new Response(null, { headers: { 'Cache-Control': 'no-store' } }),
    },
  },
})
