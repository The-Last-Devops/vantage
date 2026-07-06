<script setup>
// Presentational: renders a member's workspace-access summary as chips.
// Parent owns the data + name/label resolution.
defineProps({
  access: { type: [Array, String], default: () => [] }, // 'all' | array of { workspace_id, role }
  nameOf: { type: Function, required: true },
  wsRoleLabel: { type: Function, required: true },
})
</script>

<template>
  <span v-if="access === 'all'" class="inline-flex rounded-md border border-accent/30 bg-accent/8 px-2 py-0.5 text-xs text-accent">All workspaces</span>
  <span v-else-if="!access || !access.length" class="inline-flex rounded-md border border-dashed border-line px-2 py-0.5 text-xs text-faint">No workspaces yet</span>
  <div v-else class="flex flex-wrap gap-1.5">
    <span v-for="m in access" :key="m.workspace_id" class="inline-flex items-center gap-1 rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-fg">
      {{ nameOf(m.workspace_id) }}<span class="text-faint" :class="{ 'text-accent': m.role === 'owner', 'text-warn': m.role === 'editor' }">· {{ wsRoleLabel(m.role) }}</span><span v-if="m.can_exec" v-tip="'Shell access'" class="ml-0.5 rounded bg-accent/15 px-1 text-[10px] font-medium text-accent">shell</span>
    </span>
  </div>
</template>
