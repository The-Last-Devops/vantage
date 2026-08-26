#!/usr/bin/env node
// Unit-test the release-notes Markdown renderer. It is the only place the hub feeds
// remote text (api.github.com) into v-html, so the escaping guarantees are load-bearing:
// the XSS cases below matter more than the formatting ones.
//   node scripts/check-markdown.mjs
import { renderMarkdown } from '../frontend/src/lib/markdown.js'

let fail = 0
const t = (name, fn) => {
  try { fn(); console.log(`  ok: ${name}`) }
  catch (e) { console.log(`  FAIL: ${name}\n        ${e.message}`); fail = 1 }
}
const has = (h, needle) => {
  if (!h.includes(needle)) throw new Error(`expected to find ${JSON.stringify(needle)}\n        in ${h}`)
}
const lacks = (h, needle) => {
  if (h.includes(needle)) throw new Error(`must NOT contain ${JSON.stringify(needle)}\n        in ${h}`)
}

console.log('escaping (XSS) — the reason this file exists')
t('a script tag is text, not an element', () => {
  const h = renderMarkdown('<script>alert(1)</script>')
  lacks(h, '<script>')
  has(h, '&lt;script&gt;')
})
t('an img onerror payload is inert', () => {
  const h = renderMarkdown('<img src=x onerror=alert(1)>')
  // The payload's TEXT may appear (escaped); what must never appear is the element.
  lacks(h, '<img')
  has(h, '&lt;img src=x onerror=alert(1)&gt;')
})
t('javascript: links are not linkified', () => {
  const h = renderMarkdown('[click](javascript:alert(1))')
  lacks(h, 'href="javascript:')
  lacks(h, '<a ')
})
t('data: links are not linkified', () => {
  const h = renderMarkdown('[x](data:text/html;base64,PHNjcmlwdD4=)')
  lacks(h, '<a ')
})
t('quotes cannot break out of an attribute', () => {
  const h = renderMarkdown('[x](https://a.com"onmouseover="alert(1))')
  lacks(h, 'onmouseover="alert')
})
t('html inside a code span stays escaped', () => {
  const h = renderMarkdown('use `<b>bold</b>` here')
  lacks(h, '<b>bold</b>')
  has(h, '&lt;b&gt;bold&lt;/b&gt;')
})

console.log('formatting — what our own changelog uses')
t('headings become a styled div', () => {
  has(renderMarkdown('### Fixed'), '>Fixed<')
})
t('bullets become a list', () => {
  const h = renderMarkdown('- one\n- two')
  has(h, '<ul')
  has(h, '>one<')
  has(h, '>two<')
})
t('bold and inline code render', () => {
  const h = renderMarkdown('- **Big** thing with `some_code`')
  has(h, '<strong')
  has(h, '<code')
  has(h, 'some_code')
})
t('an http(s) link renders with noopener', () => {
  const h = renderMarkdown('see [docs](https://example.com/x)')
  has(h, 'href="https://example.com/x"')
  has(h, 'rel="noopener noreferrer"')
})
t('a wrapped bullet keeps one list item', () => {
  const h = renderMarkdown('- first line\n  continued here\n- second')
  has(h, 'first line continued here')
  if ((h.match(/<li/g) || []).length !== 2) throw new Error(`expected 2 items, got ${h}`)
})
t('asterisks inside code are not italics', () => {
  const h = renderMarkdown('`a * b`')
  lacks(h, '<em>')
})
t('blank input is empty, not a crash', () => {
  if (renderMarkdown('') !== '' || renderMarkdown(null) !== '') throw new Error('expected empty string')
})
t('our real changelog entry renders without leaking tags', () => {
  const src = [
    '## [3.0.14] - 2026-08-26',
    '',
    '### Fixed',
    '- **"Test rule" failed** with `UUID parsing failed`. It went to',
    '  `/api/alerts/[object Object]/test`.',
    '',
    'See [releases](https://github.com/x/y/releases).',
  ].join('\n')
  const h = renderMarkdown(src)
  has(h, '<strong')
  has(h, '<code')
  has(h, '<ul')
  has(h, 'href="https://github.com/x/y/releases"')
  // every < in the output must open a tag we generated
  for (const m of h.match(/<[^>]*/g) || []) {
    if (!/^<\/?(p|ul|li|div|strong|em|code|a)\b/.test(m)) throw new Error(`unexpected tag: ${m}`)
  }
})

console.log(fail ? 'FAILED' : 'PASS - renderer escapes everything it does not generate')
process.exit(fail)
