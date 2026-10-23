import { createElement } from 'react'

export function PlainText({ text }: { text: string }) {
  return createElement('span', { className: 'plain-text' }, text)
}
