export const codePoints = (text: string) => Array.from(text).length

// Unicode White_Space, matching Rust char::is_whitespace. JavaScript \s is
// different for U+0085 and U+FEFF, so it must not define the product rule.
export function validPostBody(text: string): boolean {
  const length = codePoints(text)
  return length >= 1 && length <= 280 && /[^\p{White_Space}]/u.test(text)
}
