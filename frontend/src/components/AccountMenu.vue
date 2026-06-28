<script setup>
// Account menu — moved out of the sidebar into the header right cluster. A 32px
// circular avatar (initials from the user's email) that opens a dropdown with
// the email + role, an SSH-keys link, and Logout.
import { ref, onMounted, onBeforeUnmount, computed } from 'vue'
import { useRouter, RouterLink } from 'vue-router'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const router = useRouter()

const open = ref(false)
const rootRef = ref(null)

const initials = computed(() => (auth.user?.email || '?').slice(0, 2).toUpperCase())
const role = computed(() => (auth.user?.is_admin ? 'Admin' : 'Member'))

function onDocClick(e) { if (open.value && rootRef.value && !rootRef.value.contains(e.target)) open.value = false }
onMounted(() => document.addEventListener('click', onDocClick))
onBeforeUnmount(() => document.removeEventListener('click', onDocClick))

async function logout() {
  open.value = false
  await auth.logout()
  router.push({ name: 'login' })
}
</script>

<template>
  <div ref="rootRef" class="relative">
    <button @click="open = !open" v-tip="auth.user?.email || 'Account'"
      class="grid h-8 w-8 place-items-center rounded-full bg-accent text-xs font-semibold text-accentfg hover:opacity-90">
      {{ initials }}
    </button>
    <div v-if="open" class="absolute right-0 top-full z-30 mt-1 w-56 overflow-hidden rounded-lg border border-line2 bg-surface2 py-1 shadow-xl">
      <div class="border-b border-line px-3 py-2.5">
        <div class="truncate text-sm text-fg">{{ auth.user?.email }}</div>
        <div class="text-[11px] text-faint">{{ role }}</div>
      </div>
      <RouterLink :to="{ name: 'ssh-keys' }" @click="open = false"
        class="flex items-center gap-2.5 px-3 py-2 text-sm text-muted hover:bg-surface hover:text-fg">
        <VIcon name="ssh" :size="16" class="shrink-0" />SSH keys
      </RouterLink>
      <button @click="logout"
        class="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm text-muted hover:bg-surface hover:text-fg">
        <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"/></svg>
        Logout
      </button>
    </div>
  </div>
</template>
