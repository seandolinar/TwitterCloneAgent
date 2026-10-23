import { QueryClient } from '@tanstack/react-query'
import { createApiClient } from './api.ts'

export const sessionKey = ['session'] as const

export function createQueryState(invalidateRoutes: () => Promise<void> = async () => {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        staleTime: 10_000,
        gcTime: typeof window === 'undefined' ? Infinity : 5 * 60_000,
        refetchOnWindowFocus: false,
      },
      mutations: { retry: false },
    },
  })
  const api = createApiClient(undefined, async () => {
    await queryClient.cancelQueries()
    // Keep the active session observer attached while purging other data.
    // Removing its Query would leave mounted observers displaying old identity.
    queryClient.removeQueries({ predicate: (query) => query.queryKey[0] !== sessionKey[0] })
    queryClient.getMutationCache().clear()
    queryClient.setQueryData(sessionKey, null)
    await queryClient.invalidateQueries({ queryKey: sessionKey, exact: true })
    await invalidateRoutes()
  })
  return { queryClient, api }
}
