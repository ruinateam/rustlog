<script setup lang="ts">
import { computed } from 'vue'

/** A tier such as `HT1` (high tier 1) down to `LT5` (low tier 5). */
const props = defineProps<{ tier: string }>()

// Level 1 is the best; high tiers get the stronger shade of their level.
const LEVEL_COLORS: Record<string, string> = {
  '1': 'bg-amber-400/20 text-amber-700 ring-amber-500/40 dark:text-amber-300',
  '2': 'bg-violet-400/20 text-violet-700 ring-violet-500/40 dark:text-violet-300',
  '3': 'bg-sky-400/20 text-sky-700 ring-sky-500/40 dark:text-sky-300',
  '4': 'bg-emerald-400/20 text-emerald-700 ring-emerald-500/40 dark:text-emerald-300',
  '5': 'bg-zinc-400/20 text-zinc-600 ring-zinc-500/40 dark:text-zinc-300',
}

const classes = computed(() => [
  LEVEL_COLORS[props.tier.slice(-1)] ?? LEVEL_COLORS['5'],
  props.tier.startsWith('HT') ? 'font-bold' : 'font-medium opacity-80',
])
</script>

<template>
  <span
    class="inline-flex min-w-9 justify-center rounded-md px-1.5 py-0.5 font-mono text-xs ring-1 ring-inset"
    :class="classes"
  >
    {{ tier }}
  </span>
</template>
