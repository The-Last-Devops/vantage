/** Semantic tokens backed by CSS variables → theme flips by toggling `.light`.
 *  Colours/scale kept in sync with the design system (frontend-spec/). Note: we
 *  intentionally do NOT override Tailwind's default `text-sm`/`rounded-lg` (used
 *  app-wide) — design-system sizes are added under non-colliding names. */
export default {
  content: ['./index.html', './src/**/*.{vue,js}'],
  theme: {
    extend: {
      colors: {
        bg: 'rgb(var(--bg) / <alpha-value>)',
        surface: 'rgb(var(--surface) / <alpha-value>)',
        surface2: 'rgb(var(--surface2) / <alpha-value>)',
        head: 'rgb(var(--head) / <alpha-value>)',
        line: 'rgb(var(--line) / <alpha-value>)',
        line2: 'rgb(var(--line2) / <alpha-value>)',
        hover: 'rgb(var(--hover) / <alpha-value>)',
        fg: 'rgb(var(--fg) / <alpha-value>)',
        muted: 'rgb(var(--muted) / <alpha-value>)',
        faint: 'rgb(var(--faint) / <alpha-value>)',
        cap: 'rgb(var(--cap) / <alpha-value>)',
        accent: 'rgb(var(--accent) / <alpha-value>)',
        accentfg: 'rgb(var(--accentfg) / <alpha-value>)',
        ok: 'rgb(var(--ok) / <alpha-value>)',
        warn: 'rgb(var(--warn) / <alpha-value>)',
        crit: 'rgb(var(--crit) / <alpha-value>)',
        down: 'rgb(var(--down) / <alpha-value>)',
        pending: 'rgb(var(--pending) / <alpha-value>)',
        track: 'rgb(var(--track) / <alpha-value>)',
        logobg: 'rgb(var(--logobg) / <alpha-value>)',
        logostroke: 'rgb(var(--logostroke) / <alpha-value>)',
      },
      fontFamily: {
        ui: ['Inter', 'ui-sans-serif', 'system-ui', '-apple-system', 'Segoe UI', 'Roboto', 'sans-serif'],
        mono: ['JetBrains Mono', 'ui-monospace', 'SF Mono', 'Menlo', 'monospace'],
      },
      // Design-system type scale (added; leaves Tailwind's default sizes intact).
      fontSize: {
        display: ['34px', { lineHeight: '1.05', fontWeight: '800' }],
        metric: ['28px', { lineHeight: '1.1', fontWeight: '800' }],
        h1: ['20px', { lineHeight: '1.3', fontWeight: '700' }],
        h2: ['16px', { lineHeight: '1.4', fontWeight: '600' }],
        body: ['13px', { lineHeight: '1.55' }],
        micro: ['10px', { lineHeight: '1.4' }],
      },
      borderRadius: { pill: '999px' },
      // Default border colour for a bare `border` (and `divide-*`). Without this,
      // Tailwind falls back to gray-200 (#e5e7eb) — a glaring bright line on the dark
      // UI. Point it at the subtle `--line` token so every bare border matches the
      // design system and flips with the theme.
      borderColor: { DEFAULT: 'rgb(var(--line))' },
    },
  },
  plugins: [],
}
