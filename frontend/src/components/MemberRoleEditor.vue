<script setup>
// Presentational edit slide-over: system role, per-namespace role, reset password.
// All state + RBAC/API actions live in the parent; this only renders and emits.
import UiSelect from './UiSelect.vue'

defineProps({
  member: { type: Object, required: true }, // the user being edited
  sys: { type: Array, required: true }, // [{ v, label, desc }]
  nsRoles: { type: Array, required: true }, // [{ v, label }]
  namespaces: { type: Array, default: () => [] },
  editRole: { type: String, required: true },
  editNs: { type: Object, required: true }, // namespace_id -> role ('' = no access)
  resetPw: { type: String, default: '' },
  error: { type: String, default: '' },
  initials: { type: Function, required: true },
})
const emit = defineEmits([
  'close',
  'update:editRole',
  'save-sys-role',
  'set-ns-role', // (namespace, role)
  'update:resetPw',
  'gen-password',
  'reset-password',
])
</script>

<template>
  <div class="fixed inset-0 z-50 flex justify-end bg-black/55 backdrop-blur-sm" @click.self="emit('close')">
    <aside class="flex h-full w-full max-w-[420px] flex-col border-l border-line bg-surface shadow-2xl">
      <div class="flex items-center gap-3 border-b border-line px-5 py-4">
        <span class="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-line bg-surface2 text-xs font-semibold text-muted">{{ initials(member.email) }}</span>
        <span class="min-w-0 flex-1 truncate text-sm font-medium text-fg">{{ member.email }}</span>
        <button @click="emit('close')" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
      </div>

      <div class="flex-1 space-y-6 overflow-y-auto p-5">
        <!-- system role -->
        <div>
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">System role</div>
          <UiSelect :model-value="editRole" block @update:model-value="(v) => { emit('update:editRole', v); emit('save-sys-role') }" :options="sys.map((r) => ({ value: r.v, label: r.label }))" />
          <p class="mt-1.5 text-xs text-faint">{{ sys.find((r) => r.v === editRole)?.desc }}</p>
        </div>

        <!-- namespace access -->
        <div>
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">Namespace access</div>
          <p v-if="editRole !== 'user'" class="rounded-lg border border-line bg-surface2/40 px-3 py-2.5 text-xs text-muted">{{ editRole === 'admin' ? 'Admins have full access to every namespace.' : 'Read-only admins can view every namespace.' }}</p>
          <div v-else-if="!namespaces.length" class="text-xs text-faint">No namespaces exist yet.</div>
          <div v-else class="divide-y divide-line/60">
            <div v-for="n in namespaces" :key="n.id" class="flex items-center gap-3 py-2.5">
              <span class="flex-1 truncate text-sm" :class="editNs[n.id] ? 'text-fg' : 'text-faint'">{{ n.name }}</span>
              <UiSelect :model-value="editNs[n.id]" @update:model-value="(v) => emit('set-ns-role', n, v)" class="shrink-0"
                :options="[{ value: '', label: '— no access' }, ...nsRoles.map((r) => ({ value: r.v, label: r.label }))]" />
            </div>
          </div>
        </div>

        <!-- reset password -->
        <div>
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">Reset password</div>
          <div class="flex gap-2">
            <input :value="resetPw" @input="emit('update:resetPw', $event.target.value)" type="text" placeholder="new password" class="flex-1 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
            <button @click="emit('gen-password')" class="shrink-0 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-muted hover:border-accent/50 hover:text-fg">Generate</button>
            <button @click="emit('reset-password')" class="shrink-0 rounded-lg bg-accent px-3 py-2.5 text-sm font-semibold text-accentfg hover:opacity-90">Set</button>
          </div>
        </div>

        <p v-if="error" class="text-xs" :class="error.startsWith('✓') ? 'text-accent' : 'text-rose-400'">{{ error }}</p>
      </div>

      <div class="border-t border-line px-5 py-3.5 text-center">
        <button @click="emit('close')" class="text-sm text-muted hover:text-fg">Changes save as you make them — Close</button>
      </div>
    </aside>
  </div>
</template>
