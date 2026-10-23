import { queryOptions } from '@tanstack/react-query'
import { createServerFn } from '@tanstack/react-start'
import { getRequest, setResponseHeader } from '@tanstack/react-start/server'
import { createServerApi } from './browser-proxy.server'
import { decodeResponse, type Session } from './api'
import { sessionKey } from './query-state'

const readSession = createServerFn({ method: 'GET' }).handler(async (): Promise<Session | null> => {
  setResponseHeader('Cache-Control', 'private, no-store')
  setResponseHeader('Referrer-Policy', 'no-referrer')
  const response = await createServerApi(getRequest()).read('/api/auth/session')
  if (response.status === 401) return null
  return decodeResponse<Session>(response)
})

export const sessionOptions = queryOptions({
  queryKey: sessionKey,
  queryFn: () => readSession(),
  retry: false,
  staleTime: 10_000,
})
