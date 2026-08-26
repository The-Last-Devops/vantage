// Tiny, safe-by-construction Markdown renderer for the subset our release notes use
// (headings, bullets, bold, inline code, links, paragraphs).
//
// Why not a library: the input arrives from api.github.com at runtime, so anything that
// can emit raw HTML is an XSS hole in the hub UI, and `marked` + `dompurify` is a lot of
// bytes for six constructs. This escapes EVERY character of the input FIRST and only then
// introduces tags, so nothing in the input can produce an element we did not write. Raw
// HTML in the source shows up as text rather than being interpreted.
//
// Deliberately not supported: images, tables, block quotes, code fences, HTML passthrough.

const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ESC[c])

// Only http(s) links. Anything else (javascript:, data:, vbscript:) renders as plain text.
function safeHref(url) {
  const u = url.trim()
  return /^https?:\/\/[^\s<>"']+$/i.test(u) ? u : null
}

// Sentinel that stands in for an extracted code span while the other inline rules run.
// A control character, and stripped from the input first, so it can never collide with
// real content.
const MARK = String.fromCharCode(1)

// Inline formatting, applied to ALREADY-ESCAPED text. Code spans are pulled out first so
// that ** or * inside backticks stays literal.
function inline(escaped) {
  const code = []
  let s = escaped.replace(/`([^`]+)`/g, (_, c) => {
    code.push(c)
    return MARK + (code.length - 1) + MARK
  })
  s = s.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (m, text, url) => {
    const href = safeHref(url)
    if (!href) return m
    return `<a href="${esc(href)}" target="_blank" rel="noopener noreferrer" class="text-accent hover:underline">${text}</a>`
  })
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong class="font-semibold text-fg">$1</strong>')
  s = s.replace(/(^|[\s(])\*([^*\s][^*]*)\*/g, '$1<em>$2</em>')
  return s.replace(new RegExp(MARK + '(\\d+)' + MARK, 'g'), (_, i) =>
    `<code class="rounded bg-surface2 px-1 py-0.5 font-mono text-[0.92em] text-fg">${code[Number(i)]}</code>`)
}

/** Render a Markdown subset to HTML. Safe for v-html: the input is escaped first. */
export function renderMarkdown(src) {
  const out = []
  let para = []      // pending paragraph lines
  let list = null    // pending <li> items

  const flushPara = () => {
    if (!para.length) return
    out.push(`<p class="my-2 leading-relaxed">${inline(esc(para.join(' ')))}</p>`)
    para = []
  }
  const flushList = () => {
    if (!list) return
    out.push(`<ul class="my-2 space-y-1.5 pl-4">${list.map((i) => `<li class="list-disc">${inline(esc(i))}</li>`).join('')}</ul>`)
    list = null
  }
  const flush = () => { flushPara(); flushList() }

  const lines = String(src || '')
    .replace(/\r\n?/g, '\n')
    .split(MARK).join('')
    .split('\n')

  for (const raw of lines) {
    const line = raw.trimEnd()
    if (!line.trim()) { flush(); continue }

    const h = /^(#{1,6})\s+(.*)$/.exec(line)
    if (h) {
      flush()
      const size = h[1].length <= 2 ? 'text-sm font-bold' : 'text-xs font-semibold'
      out.push(`<div class="mt-4 first:mt-0 ${size} uppercase tracking-wide text-fg">${inline(esc(h[2]))}</div>`)
      continue
    }

    const li = /^\s*[-*+]\s+(.*)$/.exec(line)
    if (li) {
      flushPara()
      list = list || []
      list.push(li[1])
      continue
    }

    // A continuation line indented under a bullet belongs to that bullet.
    if (list && /^\s{2,}\S/.test(raw)) {
      list[list.length - 1] += ' ' + line.trim()
      continue
    }

    flushList()
    para.push(line.trim())
  }
  flush()
  return out.join('')
}
