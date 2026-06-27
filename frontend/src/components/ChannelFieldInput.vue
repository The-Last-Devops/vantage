<script setup>
// One field of a notify-channel config form, rendered from a manifest FieldSpec
// (text/secret/url/number/select/toggle/textarea). Pure presentation: the value is
// bound through v-model and the reveal-secret toggle is local UI state. Secrets stay
// masked unless explicitly revealed — same behaviour as before extraction.
import { ref } from 'vue'
import UiSelect from './UiSelect.vue'

defineProps({
  field: { type: Object, required: true },
})
const model = defineModel({ default: '' })

const revealed = ref(false)
</script>

<template>
  <!-- toggle -->
  <label v-if="field.type === 'toggle'" class="flex cursor-pointer items-center gap-3">
    <input type="checkbox" class="peer sr-only" v-model="model" />
    <span class="relative h-[22px] w-10 shrink-0 rounded-full bg-line transition-colors after:absolute after:left-0.5 after:top-0.5 after:h-[18px] after:w-[18px] after:rounded-full after:bg-fg after:transition-transform peer-checked:bg-accent peer-checked:after:translate-x-[18px]"></span>
    <span>
      <span class="block text-sm text-fg">{{ field.label }}</span>
      <span v-if="field.hint" class="block text-xs text-faint">{{ field.hint }}</span>
    </span>
  </label>
  <!-- everything else -->
  <label v-else class="block">
    <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">{{ field.label }}<span v-if="field.required" class="ml-0.5 text-rose-400">*</span></span>
    <UiSelect v-if="field.type === 'select'" v-model="model" block :options="field.options" />
    <textarea v-else-if="field.type === 'textarea'" v-model="model" :placeholder="field.placeholder" rows="3" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none"></textarea>
    <span v-else-if="field.type === 'secret'" class="relative block">
      <input :type="revealed ? 'text' : 'password'" v-model="model" :placeholder="field.placeholder" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 pr-10 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
      <button type="button" @click="revealed = !revealed" class="absolute right-1.5 top-1.5 rounded p-1.5 text-faint hover:text-fg" :class="revealed && 'text-accent'" aria-label="Show">
        <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>
      </button>
    </span>
    <input v-else :type="field.type === 'number' ? 'number' : 'text'" v-model="model" :placeholder="field.placeholder" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
    <span v-if="field.hint" class="mt-1.5 block text-xs text-faint">{{ field.hint }}</span>
  </label>
</template>
