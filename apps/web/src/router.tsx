import { createRouter } from '@tanstack/react-router'
import { setupRouterSsrQueryIntegration } from '@tanstack/react-router-ssr-query'
import { createIsomorphicFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'
import { nonceFor } from './lib/security.server'
import { createQueryState } from './lib/query-state'
import { routeTree } from './routeTree.gen'

const requestNonce = createIsomorphicFn()
  .server(() => nonceFor(getRequest()))
  .client(() => document.querySelector<HTMLScriptElement>('script[nonce]')?.nonce)

export function getRouter() {
  const state = createQueryState(async () => { await router.invalidate() })
  const router = createRouter({
    routeTree,
    context: state,
    scrollRestoration: true,
    defaultPreload: 'intent',
    defaultPreloadStaleTime: 0,
    ssr: { nonce: requestNonce() },
  })
  setupRouterSsrQueryIntegration({ router, queryClient: state.queryClient })
  return router
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}
