<script setup>
// Overview "What is wrong, exactly" — the rows BEHIND the action tiles. A tile says
// "2 hosts down"; this says which two, for how long, and where to click. Four cards
// (hosts down · over threshold · services down · alerts firing); a card with nothing
// in it is not rendered, and when all four are empty the whole block collapses to a
// single "all clear" line. Each card shows at most MAX rows and links to the filtered
// list page for the rest, so the Overview stays one screen whatever the fleet size.
// Rows are plain data computed by Overview.vue — this component only renders.
const MAX = 5
defineProps({
  // [{ id, name, ws, detail, right, sub, to }] — `right` carries the tone colour.
  hostsDown: { type: Array, default: () => [] },
  hostsOver: { type: Array, default: () => [] }, // + tone: 'crit' | 'warn'
  servicesDown: { type: Array, default: () => [] },
  servicesPending: { type: Number, default: 0 },
  alertsFiring: { type: Array, default: () => [] },
  // Where "view all" goes for each card.
  links: { type: Object, default: () => ({}) },
  totals: { type: Object, default: () => ({}) }, // { hosts, services }
})
const TEXT = { down: 'text-down', crit: 'text-crit', warn: 'text-warn' }
const DOT = { down: 'bg-down', crit: 'bg-crit', warn: 'bg-warn' }
</script>

<template>
  <section>
    <h2 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-faint">Incidents</h2>

    <div v-if="!hostsDown.length && !hostsOver.length && !servicesDown.length && !alertsFiring.length"
      class="flex items-center gap-3 rounded-xl border border-ok/35 bg-ok/10 px-4 py-3">
      <span class="h-2 w-2 shrink-0 rounded-full bg-ok shadow-[0_0_0_3px_rgb(var(--ok)/0.2)]"></span>
      <div>
        <div class="text-h2 font-semibold text-ok">All clear</div>
        <div class="text-xs text-faint">{{ totals.hosts || 0 }} hosts reporting · {{ totals.services || 0 }} services up · no alerts firing</div>
      </div>
    </div>

    <!-- CSS columns, not a grid: a grid row is as tall as its tallest card, so one
         host down next to eight hosts over threshold left a card-sized hole (seen in
         production). Columns let each card take its own height and the next one move up. -->
    <div v-else class="lg:columns-2 lg:gap-3">
      <template v-for="card in [
        { key: 'hd', title: 'Hosts down', tone: 'down', rows: hostsDown, count: `${hostsDown.length} / ${totals.hosts || 0}`, to: links.hostsDown },
        { key: 'ho', title: 'Over threshold', tone: 'crit', rows: hostsOver, count: `${hostsOver.filter((r) => r.tone === 'crit').length} crit · ${hostsOver.filter((r) => r.tone === 'warn').length} warn`, to: links.hostsOver },
        { key: 'sd', title: 'Services down', tone: 'down', rows: servicesDown, count: `${servicesDown.length} / ${totals.services || 0}`, to: links.servicesDown, foot: servicesPending ? `${servicesPending} pending (no check yet)` : '' },
        { key: 'af', title: 'Alerts firing', tone: 'down', rows: alertsFiring, count: `${alertsFiring.length} rules`, to: links.alertsFiring },
      ]" :key="card.key">
        <div v-if="card.rows.length" class="mb-3 break-inside-avoid overflow-hidden rounded-xl border border-line bg-surface">
          <div class="flex items-center gap-2 border-b border-line bg-head px-3.5 py-2 text-xs font-semibold text-fg">
            <span class="h-2 w-2 shrink-0 rounded-full" :class="DOT[card.tone]"></span>{{ card.title }}
            <span class="ml-auto text-[11px] font-medium text-faint">{{ card.count }} ·
              <RouterLink :to="card.to" class="text-accent hover:underline">view all</RouterLink></span>
          </div>
          <RouterLink v-for="r in card.rows.slice(0, MAX)" :key="r.id" :to="r.to"
            class="grid grid-cols-[10px_1fr_auto] items-center gap-2.5 border-t border-line px-3.5 py-2 first:border-t-0 hover:bg-hover">
            <span class="h-2 w-2 rounded-full" :class="DOT[r.tone || card.tone]"></span>
            <div class="min-w-0">
              <div class="truncate font-mono text-body font-semibold text-fg">{{ r.name }}
                <span v-if="r.ws" class="ml-1.5 rounded-pill border border-line2 px-1.5 text-micro font-normal text-muted">{{ r.ws }}</span>
              </div>
              <div class="truncate text-[11px] text-faint">{{ r.detail }}</div>
            </div>
            <div class="whitespace-nowrap text-right text-xs">
              <b class="block font-bold" :class="TEXT[r.tone || card.tone]">{{ r.right }}</b>
              <small v-if="r.sub" class="text-[11px] text-faint">{{ r.sub }}</small>
            </div>
          </RouterLink>
          <div v-if="card.rows.length > MAX || card.foot" class="border-t border-line px-3.5 py-1.5 text-xs text-faint">
            <RouterLink v-if="card.rows.length > MAX" :to="card.to" class="text-accent hover:underline">+{{ card.rows.length - MAX }} more</RouterLink>
            <span v-if="card.rows.length > MAX && card.foot"> · </span>{{ card.foot }}
          </div>
        </div>
      </template>
    </div>
  </section>
</template>
