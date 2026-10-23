import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { useRouter } from '@tanstack/react-router'
import { ApiFailure } from '../lib/api'
import { sessionOptions } from '../lib/session'

export function useAccount() {
  const router = useRouter()
  const { api, queryClient } = router.options.context!
  return {
    api,
    async refreshSession() {
      await queryClient.invalidateQueries({ queryKey: sessionOptions.queryKey })
      await router.invalidate()
    },
  }
}

export function passwordFrom(form: FormData, name = 'password'): string {
  const password = String(form.get(name) ?? '')
  if (Array.from(password).length < 15 || Array.from(password).length > 128) {
    throw new ApiFailure(422, 'invalid_password', 'Passwords require 15–128 Unicode code points.')
  }
  return password
}

export function tokenSearch(search: Record<string, unknown>): { token?: string } {
  return typeof search.token === 'string' ? { token: search.token } : {}
}

export function AccountForm({ children, submit, label, success, afterSuccess, disabled = false }: {
  children?: ReactNode
  submit: (form: FormData) => Promise<void>
  label: string
  success: string
  afterSuccess?: () => Promise<void>
  disabled?: boolean
}) {
  const [hydrated, setHydrated] = useState(false)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState('')
  const [message, setMessage] = useState('')
  const errorRef = useRef<HTMLParagraphElement>(null)
  const feedbackId = useId()
  useEffect(() => { setHydrated(true) }, [])
  useEffect(() => { if (error) errorRef.current?.focus() }, [error])
  return <form method="post" aria-busy={pending} aria-describedby={feedbackId} onSubmit={async (event) => {
    event.preventDefault()
    if (pending) return
    const form = event.currentTarget
    const input = new FormData(form)
    setPending(true)
    setError('')
    setMessage('')
    try {
      await submit(input)
      form.reset()
      setMessage(success)
      await afterSuccess?.()
    } catch (error) {
      setError(error instanceof ApiFailure ? error.message : 'The service could not be reached. Try again.')
    } finally {
      form.querySelectorAll<HTMLInputElement>('input[type="password"]').forEach((input) => { input.value = '' })
      setPending(false)
    }
  }}>
    <fieldset disabled={!hydrated || pending || disabled}>
      {children}
      <button type="submit">{pending ? 'Please wait…' : label}</button>
    </fieldset>
    <div id={feedbackId} aria-live="polite">
      {error && <p className="form-error" role="alert" tabIndex={-1} ref={errorRef}>{error}</p>}
      {message && <p role="status">{message}</p>}
    </div>
    {!hydrated && <p>These secure forms require JavaScript. Enable it to continue.</p>}
  </form>
}

export function Field({ label, name, type = 'text', autoComplete, hint, pattern }: {
  label: string
  name: string
  type?: 'text' | 'email' | 'password'
  autoComplete: string
  hint?: string
  pattern?: string
}) {
  const id = useId()
  return <div className="field">
    <label htmlFor={id}>{label}</label>
    <input id={id} name={name} type={type} autoComplete={autoComplete} required
      pattern={pattern} aria-describedby={hint ? id + '-hint' : undefined} />
    {hint && <small id={id + '-hint'}>{hint}</small>}
  </div>
}
