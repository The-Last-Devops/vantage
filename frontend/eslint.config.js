// Minimal lint with ONE job: catch the mistakes a `vite build` happily compiles and
// that only surface as a broken page at runtime. Not a style checker — no formatting
// rules, no opinions, so it stays quiet unless something is actually wrong.
//
// It exists because `` `/api/alerts/${editId}/test` `` shipped in 3.0.13: editId is a
// computed ref, so the URL interpolated to "[object Object]" and Test rule 400'd. The
// build was green and the browser console check saw nothing (the error was caught and
// rendered as UI text).
import vue from 'eslint-plugin-vue'

export default [
  ...vue.configs['flat/essential'],
  {
    files: ['**/*.{js,vue}'],
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: 'module',
      globals: { window: 'readonly', document: 'readonly', navigator: 'readonly',
                 console: 'readonly', fetch: 'readonly', localStorage: 'readonly',
                 sessionStorage: 'readonly', setTimeout: 'readonly', clearTimeout: 'readonly',
                 setInterval: 'readonly', clearInterval: 'readonly', location: 'readonly',
                 alert: 'readonly', crypto: 'readonly', URL: 'readonly', Blob: 'readonly',
                 requestAnimationFrame: 'readonly', ResizeObserver: 'readonly',
                 WebSocket: 'readonly', atob: 'readonly', btoa: 'readonly',
                 TextEncoder: 'readonly', TextDecoder: 'readonly', Image: 'readonly',
                 performance: 'readonly', history: 'readonly', getComputedStyle: 'readonly',
                 matchMedia: 'readonly', AbortController: 'readonly', Event: 'readonly',
                 CustomEvent: 'readonly', MutationObserver: 'readonly', queueMicrotask: 'readonly',
                 FileReader: 'readonly', FormData: 'readonly', Notification: 'readonly',
                 URLSearchParams: 'readonly', DOMParser: 'readonly', Element: 'readonly' },
    },
    rules: {
      // The one that would have caught the 3.0.13 bug: a ref used where its value was
      // meant (`!someRef`, `${someRef}`, arithmetic on a ref).
      'vue/no-ref-as-operand': 'error',
      'no-undef': 'error',
      'no-unused-vars': ['warn', { args: 'none', varsIgnorePattern: '^_' }],
      // Pure style, and every page component here is deliberately one word.
      'vue/multi-word-component-names': 'off',
      // Real anti-pattern, but ChannelForm has been v-model-ing into a prop object
      // since it was written and untangling it is its own change — warn, don't block.
      'vue/no-mutating-props': 'warn',
    },
  },
  { ignores: ['dist/**', 'node_modules/**'] },
]
