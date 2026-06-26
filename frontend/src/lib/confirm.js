// Themed replacement for window.confirm(): `await confirm({ ... })` resolves true
// (confirmed) or false (cancelled). A single <ConfirmDialog/> (mounted in App.vue)
// renders this shared state.
import { reactive } from 'vue'

export const confirmState = reactive({
  open: false,
  title: 'Are you sure?',
  message: '',
  confirmText: 'Confirm',
  cancelText: 'Cancel',
  danger: false,
  _resolve: null,
})

/**
 * Ask the user to confirm. Returns a Promise<boolean>.
 * opts: { title, message, danger, confirmText, cancelText }
 */
export function confirm(opts = {}) {
  // If a previous prompt is somehow still open, resolve it as cancelled first.
  if (confirmState._resolve) confirmState._resolve(false)
  return new Promise((resolve) => {
    confirmState.title = opts.title || 'Are you sure?'
    confirmState.message = opts.message || ''
    confirmState.danger = !!opts.danger
    confirmState.confirmText = opts.confirmText || (opts.danger ? 'Delete' : 'Confirm')
    confirmState.cancelText = opts.cancelText || 'Cancel'
    confirmState._resolve = resolve
    confirmState.open = true
  })
}

export function settleConfirm(value) {
  const r = confirmState._resolve
  confirmState._resolve = null
  confirmState.open = false
  if (r) r(value)
}
