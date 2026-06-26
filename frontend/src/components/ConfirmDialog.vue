<script setup>
// Renders the shared confirm() state (lib/confirm.js). Mounted once in App.vue.
import { watch, nextTick, ref, onMounted, onBeforeUnmount } from 'vue'
import { confirmState, settleConfirm } from '../lib/confirm'

const okBtn = ref(null)

function onKey(e) {
  if (!confirmState.open) return
  if (e.key === 'Escape') { e.preventDefault(); settleConfirm(false) }
  else if (e.key === 'Enter') { e.preventDefault(); settleConfirm(true) }
}
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))

// Focus the confirm button when it opens (Enter then confirms).
watch(() => confirmState.open, async (open) => {
  if (open) { await nextTick(); okBtn.value?.focus() }
})
</script>

<template>
  <Teleport to="body">
    <div v-if="confirmState.open" class="fixed inset-0 z-[120] grid place-items-center bg-black/60 p-5 backdrop-blur-sm"
      @click.self="settleConfirm(false)">
      <div class="w-full max-w-md overflow-hidden rounded-2xl border border-line bg-surface shadow-[0_24px_60px_-16px_rgba(0,0,0,0.7)]" role="alertdialog" aria-modal="true">
        <div class="flex gap-4 px-6 pb-2 pt-6">
          <span class="grid h-11 w-11 shrink-0 place-items-center rounded-xl" :class="confirmState.danger ? 'bg-rose-500/14 text-rose-400' : 'bg-accent/14 text-accent'">
            <svg v-if="confirmState.danger" class="h-6 w-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m10.29 3.86-8.48 14.7A2 2 0 0 0 3.53 21h16.94a2 2 0 0 0 1.72-2.44L13.71 3.86a2 2 0 0 0-3.42 0Z"/><path d="M12 9v4M12 17h.01"/></svg>
            <svg v-else class="h-6 w-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3"/><path d="M12 17h.01"/></svg>
          </span>
          <div class="min-w-0">
            <h3 class="mb-1.5 mt-0.5 text-base font-semibold text-fg">{{ confirmState.title }}</h3>
            <p v-if="confirmState.message" class="text-[13.5px] leading-relaxed text-muted">{{ confirmState.message }}</p>
          </div>
        </div>
        <div class="flex justify-end gap-2.5 px-6 pb-5 pt-4">
          <button @click="settleConfirm(false)" class="rounded-lg px-3 py-2 text-sm font-medium text-muted hover:text-fg">{{ confirmState.cancelText }}</button>
          <button ref="okBtn" @click="settleConfirm(true)"
            class="rounded-lg px-4 py-2 text-sm font-bold focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-offset-surface"
            :class="confirmState.danger ? 'bg-rose-500 text-rose-950 hover:bg-rose-400 focus:ring-rose-500/50' : 'bg-accent text-accentfg hover:opacity-90 focus:ring-accent/50'">
            {{ confirmState.confirmText }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
