<script setup>
// Presentational edit slide-over: system role, per-workspace role, reset password.
// All state + RBAC/API actions live in the parent; this only renders and emits.
import UiSelect from './UiSelect.vue'

defineProps({
  dirty: { type: Boolean, default: false },
  saving: { type: Boolean, default: false },
  member: { type: Object, required: true }, // the user being edited
  sys: { type: Array, required: true }, // [{ v, label, desc }]
  wsRoles: { type: Array, required: true }, // [{ v, label }]
  workspaces: { type: Array, default: () => [] },
  editRole: { type: String, required: true },
  editWs: { type: Object, required: true }, // workspace_id -> role ('' = no access)
  editWsExec: { type: Object, default: () => ({}) }, // workspace_id -> can_exec (shell)
  resetPw: { type: String, default: '' },
  error: { type: String, default: '' },
  initials: { type: Function, required: true },
})
// Nothing here calls the API. Edits build up a draft in the parent and only leave
// the browser when Save is pressed — changing someone's role used to fire a request
// on every keystroke of the dropdown, so a mis-click was already live.
const emit = defineEmits([
  'close',
  'save',
  'update:editRole',
  'set-ws-role', // (workspace, role) — draft only
  'set-ws-exec', // (workspace, can_exec) — draft only
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
          <UiSelect :model-value="editRole" block @update:model-value="(v) => emit('update:editRole', v)" :options="sys.map((r) => ({ value: r.v, label: r.label }))" />
          <p class="mt-1.5 text-xs text-faint">{{ sys.find((r) => r.v === editRole)?.desc }}</p>
        </div>

        <!-- workspace access -->
        <div>
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">Workspace access</div>
          <!-- Only a full admin has nothing to configure here. A read-only admin reads
               every workspace as a FLOOR, and can still be given a real role on the ones
               they work in (rbac::role_in) — so they keep the picker. -->
          <p v-if="editRole === 'admin'" class="rounded-lg border border-line bg-surface2/40 px-3 py-2.5 text-xs text-muted">Admins have full access to every workspace.</p>
          <template v-else>
            <p v-if="editRole === 'read_all'" class="mb-2 rounded-lg border border-line bg-surface2/40 px-3 py-2.5 text-xs text-muted">Read-only admins can view every workspace. Give them a role below on the ones where they should be able to make changes.</p>
            <div v-if="!workspaces.length" class="text-xs text-faint">No workspaces exist yet.</div>
          <div v-else class="divide-y divide-line/60">
            <div v-for="n in workspaces" :key="n.id" class="py-2.5">
              <div class="flex items-center gap-3">
                <span class="flex-1 truncate text-sm" :class="editWs[n.id] ? 'text-fg' : 'text-faint'">{{ n.name }}</span>
                <UiSelect :model-value="editWs[n.id]" @update:model-value="(v) => emit('set-ws-role', n, v)" class="shrink-0"
                  :options="[{ value: '', label: '— no access' }, ...wsRoles.map((r) => ({ value: r.v, label: r.label }))]" />
              </div>
              <!-- Remote SSH is a SEPARATE grant on top of owner (rbac::require_exec needs
                   both). Always rendered — when the member is not an owner it stays visible
                   but disabled, because a silently absent checkbox reads as "this product has
                   no such permission". -->
              <label v-if="editWs[n.id]" class="mt-2 flex items-start gap-2 pl-0.5 text-xs"
                :class="editWs[n.id] === 'owner' ? 'text-muted' : 'text-faint'">
                <input type="checkbox" class="mt-0.5 h-3.5 w-3.5 accent-accent disabled:opacity-40"
                  :checked="!!editWsExec[n.id]" :disabled="editWs[n.id] !== 'owner'"
                  @change="emit('set-ws-exec', n, $event.target.checked)" />
                <span v-if="editWs[n.id] === 'owner'">Remote SSH <span class="text-faint">— open an interactive console on this workspace's hosts</span></span>
                <span v-else>Remote SSH <span class="text-faint">— requires the <b>owner</b> role in this workspace</span></span>
              </label>
            </div>
            </div>
          </template>
        </div>

        <!-- reset password — its own explicit action, not part of the draft -->
        <div>
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">Reset password</div>
          <div class="flex gap-2">
            <input :value="resetPw" @input="emit('update:resetPw', $event.target.value)" type="text" placeholder="new password" class="flex-1 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
            <button @click="emit('gen-password')" class="shrink-0 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-muted hover:border-accent/50 hover:text-fg">Generate</button>
            <button @click="emit('reset-password')" class="shrink-0 rounded-lg bg-accent px-3 py-2.5 text-sm font-semibold text-accentfg hover:opacity-90">Set</button>
          </div>
        </div>

        <p v-if="error" class="text-xs" :class="error.startsWith('✓') ? 'text-accent' : 'text-down'">{{ error }}</p>
      </div>

      <div class="flex items-center gap-2.5 border-t border-line px-5 py-3.5">
        <span v-if="dirty" class="text-xs text-muted">Unsaved changes</span>
        <span class="ml-auto"></span>
        <button @click="emit('close')" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">{{ dirty ? 'Cancel' : 'Close' }}</button>
        <button @click="emit('save')" :disabled="!dirty || saving"
          class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-40">{{ saving ? 'Saving…' : 'Save changes' }}</button>
      </div>
    </aside>
  </div>
</template>
