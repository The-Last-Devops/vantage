<script setup>
// The name cell of a notify-channel row: the provider glyph + the channel name.
// Pure presentation — the parent owns the data and the icon map, passing the
// pre-rendered glyph SVG (`iconHtml`) plus its colours so the icon logic stays in
// one place. (The Test/Edit/Delete actions stay in the parent: they're bound to
// parent-only test-result state and handlers.)
defineProps({
  name: { type: String, default: '' },
  iconHtml: { type: String, default: '' }, // rendered <svg> for the channel kind
  iconColor: { type: String, default: 'rgb(var(--surface2))' },
  iconFg: { type: String, default: 'rgb(var(--fg))' },
})
</script>

<template>
  <div class="flex items-center gap-2.5">
    <span class="grid h-7 w-7 shrink-0 place-items-center rounded-lg" :style="{ background: iconColor, color: iconFg }" v-html="iconHtml"></span>
    <span class="font-medium text-fg">{{ name }}</span>
  </div>
</template>
