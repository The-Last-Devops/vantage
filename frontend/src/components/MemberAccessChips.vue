<script setup>
// Presentational: renders a member's namespace-access summary as chips.
// Parent owns the data + name/label resolution.
defineProps({
  access: { type: [Array, String], default: () => [] }, // 'all' | array of { namespace_id, role }
  nameOf: { type: Function, required: true },
  nsRoleLabel: { type: Function, required: true },
})
</script>

<template>
  <span v-if="access === 'all'" class="inline-flex rounded-md border border-accent/30 bg-accent/8 px-2 py-0.5 text-xs text-accent">All namespaces</span>
  <span v-else-if="!access || !access.length" class="inline-flex rounded-md border border-dashed border-line px-2 py-0.5 text-xs text-faint">No namespaces yet</span>
  <div v-else class="flex flex-wrap gap-1.5">
    <span v-for="m in access" :key="m.namespace_id" class="inline-flex items-center gap-1 rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-fg">
      {{ nameOf(m.namespace_id) }}<span class="text-faint" :class="{ 'text-accent': m.role === 'owner', 'text-amber-400': m.role === 'editor' }">· {{ nsRoleLabel(m.role) }}</span>
    </span>
  </div>
</template>
