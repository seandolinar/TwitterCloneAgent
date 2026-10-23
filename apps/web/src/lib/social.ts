import { infiniteQueryOptions, queryOptions } from '@tanstack/react-query'
import { createServerFn } from '@tanstack/react-start'
import { getRequest, setResponseHeader } from '@tanstack/react-start/server'
import { createServerApi } from './browser-proxy.server'
import { ApiFailure, decodeResponse, type ApiClient, type Page, type Post, type Profile } from './api'

type SocialPath = `/api/v1/${string}`

// Only safe read results cross SSR serialization; errors retain their status
// without exposing a framework stack, a cookie, or an arbitrary backend body.
const serverRead = createServerFn({ method: 'GET' })
  .validator((input: { path: string }) => {
    if (typeof input.path !== 'string' || !input.path.startsWith('/api/v1/')) {
      throw new Error('Invalid social read')
    }
    return input
  })
  .handler(async ({ data }) => {
    setResponseHeader('Cache-Control', 'private, no-store')
    const response = await createServerApi(getRequest()).read(data.path)
    try {
      return { ok: true as const, value: await decodeResponse<Profile | Post | Page>(response) }
    } catch (error) {
      if (!(error instanceof ApiFailure)) throw error
      return { ok: false as const, status: error.status, code: error.code, message: error.message }
    }
  })

async function read<T>(api: ApiClient, path: SocialPath, signal?: AbortSignal): Promise<T> {
  if (typeof window !== 'undefined') return api.read<T>(path, signal)
  const result = await serverRead({ data: { path } })
  if (!result.ok) throw new ApiFailure(result.status, result.code, result.message)
  return result.value as T
}

export const socialKey = ['social'] as const
export const profileOptions = (api: ApiClient, handle: string) => queryOptions({
  queryKey: [...socialKey, 'profile', handle.toLowerCase()],
  queryFn: ({ signal }) => read<Profile>(api, `/api/v1/profiles/${handle}`, signal),
})
export const postOptions = (api: ApiClient, id: string) => queryOptions({
  queryKey: [...socialKey, 'post', id],
  queryFn: ({ signal }) => read<Post>(api, `/api/v1/posts/${id}`, signal),
})
export const timelineOptions = (api: ApiClient, path: SocialPath) => infiniteQueryOptions({
  queryKey: [...socialKey, 'timeline', path],
  initialPageParam: null as string | null,
  queryFn: ({ pageParam, signal }) => {
    const query = new URLSearchParams({ limit: '20' })
    if (pageParam !== null) query.set('cursor', pageParam)
    return read<Page>(api, `${path}?${query}`, signal)
  },
  getNextPageParam: (page: Page) => page.nextCursor ?? undefined,
})

export function uniquePosts(pages: Page[]): Post[] {
  const seen = new Set<string>()
  return pages.flatMap((page) => page.items).filter((post) => {
    if (seen.has(post.id)) return false
    seen.add(post.id)
    return true
  })
}

export function socialError(error: unknown): string {
  if (error instanceof ApiFailure) {
    if (error.status === 429) return `${error.message} Try again${error.retryAfter ? ` in ${error.retryAfter} seconds` : ' later'}.`
    return error.message
  }
  return 'The service could not be reached. Check your connection and try again.'
}
