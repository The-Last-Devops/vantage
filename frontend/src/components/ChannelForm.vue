<script setup>
// The dynamic notify-channel config form: a workspace picker (create only), a name,
// then the provider's basic + advanced fields rendered from the manifest. Pure
// presentation — fetching/state/validation live in the parent (Notifications.vue).
// Field values are written straight into the shared `config` object (v-model on each
// ChannelFieldInput), and `name`/`wsId` are bound through their own v-models.
import { computed } from 'vue'
import UiSelect from './UiSelect.vue'
import ChannelFieldInput from './ChannelFieldInput.vue'

const props = defineProps({
  cur: { type: Object, default: null }, // current provider meta
  config: { type: Object, required: true }, // form.config (mutated in place)
  workspaces: { type: Array, default: () => [] },
  editId: { type: [Number, String, null], default: null },
  readOnly: { type: Boolean, default: false },
})
const name = defineModel('name', { default: '' })
const wsId = defineModel('wsId', { default: '' })

const basicFields = computed(() => props.cur?.fields.filter((f) => !f.advanced) || [])
const advFields = computed(() => props.cur?.fields.filter((f) => f.advanced) || [])
</script>

<template>
  <fieldset :disabled="readOnly" class="max-h-[60vh] space-y-4 overflow-auto p-5">
    <p v-if="readOnly" class="rounded-lg border border-line bg-surface2/50 px-3 py-2 text-xs text-muted">View only — this channel belongs to another workspace. Editors of that workspace can change it.</p>
    <label v-if="!editId" class="block">
      <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Workspace</span>
      <UiSelect v-model="wsId" block :options="workspaces.map((n) => ({ value: n.id, label: n.name }))" />
      <span class="mt-1.5 block text-xs text-faint">The channel lives here; only editors of this workspace can change it later. Any alert can still use it.</span>
    </label>
    <label class="block">
      <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Name</span>
      <input v-model="name" placeholder="e.g. ops-alerts" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
    </label>

    <!-- basic fields -->
    <div v-for="f in basicFields" :key="f.key">
      <ChannelFieldInput :field="f" v-model="config[f.key]" />
    </div>

    <!-- advanced (always expanded — plenty of room, nothing to hide) -->
    <div v-if="advFields.length" class="border-t border-line/70 pt-4">
      <div class="mb-3 text-[11px] font-semibold uppercase tracking-wide text-faint">Advanced options <span class="font-normal normal-case text-faint/70">· optional</span></div>
      <div class="space-y-4">
        <div v-for="f in advFields" :key="f.key">
          <ChannelFieldInput :field="f" v-model="config[f.key]" />
        </div>
      </div>
    </div>
  </fieldset>
</template>
