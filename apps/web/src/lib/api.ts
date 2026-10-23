// Mirrors the private Rust DTOs. No credential values belong in query data.
export interface Session {
  id: string
  handle: string
  verified: boolean
  recentPasswordAuth: boolean
}

export interface Profile {
  id: string
  handle: string
  displayName: string
  biography: string
  joinedAt: string
  followerCount: number
  followingCount: number
  postCount: number
  viewerFollows: boolean
}

export interface Post {
  id: string
  authorId: string
  authorHandle: string
  authorDisplayName: string
  body: string
  createdAt: string
  likeCount: number
  viewerLikes: boolean
}

export interface Page {
  items: Post[]
  nextCursor: string | null
}

export interface KeyMetadata {
  keyId: string
  name: string
  maskedHint: string
  permissions: string[]
  createdAt: string
  expiresAt: string
  revokedAt: string | null
  providerRevokedAt: string | null
  status: 'active' | 'expired' | 'revoked_pending_cleanup' | 'revoked'
}

export interface KeyList {
  integrationAvailable: boolean
  items: KeyMetadata[]
}

export class ApiFailure extends Error {
  readonly status: number
  readonly code: string
  readonly requestId?: string
  readonly retryAfter?: number
  constructor(status: number, code: string, message: string, requestId?: string, retryAfter?: number) {
    super(message)
    this.name = 'ApiFailure'
    this.status = status
    this.code = code
    this.requestId = requestId
    this.retryAfter = retryAfter
  }
}

export async function decodeResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let value: { error?: { code?: string; message?: string }; requestId?: string } = {}
    try { value = await response.json() } catch { /* safe fallback, never show a raw error body */ }
    throw new ApiFailure(response.status, value.error?.code ?? 'request_failed',
      value.error?.message ?? 'The request could not be completed.', value.requestId,
      Number(response.headers.get('Retry-After')) || undefined)
  }
  if (response.status === 204 || response.status === 202) return undefined as T
  return response.json() as Promise<T>
}

type ApiPath = `/api/${string}`
type MutationMethod = 'POST' | 'PUT' | 'PATCH' | 'DELETE'
type Fetcher = (input: string, init?: RequestInit) => Promise<Response>

function safePath(path: string): asserts path is ApiPath {
  const pathname = path.split('?', 1)[0]
  if (!pathname.startsWith('/api/') || /[%\\#\x00-\x20]/.test(pathname) ||
    pathname.includes('//') || pathname.split('/').some((part) => part === '.' || part === '..')) {
    throw new ApiFailure(400, 'invalid_path', 'The request path is not allowed.')
  }
}

// One instance per router/browser application, never a module-level singleton.
// Raw CSRF values stay only in this closure; they are not serialized or persisted.
export function createApiClient(
  fetcher: Fetcher = (path, init) => fetch(path, init),
  identityChanged: () => Promise<void> = async () => {},
) {
  let csrf: string | undefined
  let bootstrap: Promise<void> | undefined
  async function refreshCsrf(): Promise<void> {
    bootstrap ??= (async () => {
      const result = await decodeResponse<{ csrfToken: string }>(
        await fetcher('/api/auth/csrf', { credentials: 'same-origin', cache: 'no-store' }),
      )
      if (typeof result.csrfToken !== 'string') {
        throw new ApiFailure(503, 'invalid_response', 'The service is temporarily unavailable.')
      }
      csrf = result.csrfToken
    })().finally(() => { bootstrap = undefined })
    return bootstrap
  }
  async function read<T>(path: ApiPath, signal?: AbortSignal): Promise<T> {
    safePath(path)
    return decodeResponse<T>(await fetcher(path, { signal, credentials: 'same-origin', cache: 'no-store' }))
  }
  async function mutate<T>(path: ApiPath, method: MutationMethod, body?: unknown): Promise<T> {
    safePath(path)
    if (!csrf) await refreshCsrf()
    const send = () => fetcher(path, {
      method, credentials: 'same-origin', cache: 'no-store',
      headers: { 'X-CSRF-Token': csrf!, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    let response = await send()
    // Retry only a confirmed pre-mutation CSRF rejection, once. Never retry
    // writes on network errors/timeouts or an ambiguous application response.
    if (response.status === 403) {
      const error = await response.clone().json().catch(() => undefined)
      if (error?.error?.code === 'csrf_rejected') {
        await refreshCsrf()
        response = await send()
      }
    }
    const result = await decodeResponse<T>(response)
    if (result && typeof result === 'object' && 'csrfToken' in result) {
      const token = (result as { csrfToken: unknown }).csrfToken
      if (typeof token === 'string') csrf = token
    }
    if (['/api/auth/login', '/api/auth/logout', '/api/auth/reset-password'].includes(path)) {
      await identityChanged()
    }
    return result
  }
  return {
    read, mutate, refreshCsrf,
    register: (input: { email: string; handle: string; password: string }) =>
      mutate<void>('/api/auth/register', 'POST', input),
    login: async (input: { email: string; password: string }) => {
      await mutate<{ csrfToken: string }>('/api/auth/login', 'POST', input)
    },
    logout: async () => { await mutate<{ csrfToken: string }>('/api/auth/logout', 'POST') },
    verifyEmail: (token: string) => mutate<void>('/api/auth/verify-email', 'POST', { token }),
    resendVerification: () => mutate<void>('/api/auth/resend-verification', 'POST'),
    requestReset: (email: string) => mutate<void>('/api/auth/forgot-password', 'POST', { email }),
    resetPassword: async (token: string, password: string) => {
      await mutate<{ csrfToken: string }>('/api/auth/reset-password', 'POST', { token, password })
    },
    reauthenticate: (password: string) => mutate<void>('/api/auth/reauthenticate', 'POST', { password }),
  }
}

export type ApiClient = ReturnType<typeof createApiClient>
