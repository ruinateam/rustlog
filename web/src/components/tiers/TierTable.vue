<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { Schemas } from '@/api/client'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import TierBadge from './TierBadge.vue'

defineProps<{ entries: Schemas['TierTableEntry'][]; channel: string }>()
const { t, n } = useI18n()

const WINDOWS = ['1m', '5m', '15m', '30m', '60m'] as const
</script>

<template>
  <Table>
    <TableHeader>
      <TableRow>
        <TableHead class="w-10 text-right">{{ t('tiers.rank') }}</TableHead>
        <TableHead>{{ t('common.user') }}</TableHead>
        <TableHead v-for="window in WINDOWS" :key="window" class="text-center">{{ window }}</TableHead>
        <TableHead class="text-right">{{ t('tiers.score') }}</TableHead>
        <TableHead class="hidden text-right sm:table-cell">{{ t('tiers.messages') }}</TableHead>
        <TableHead class="hidden text-right md:table-cell">{{ t('tiers.unique') }}</TableHead>
      </TableRow>
    </TableHeader>
    <TableBody>
      <TableRow v-for="(entry, index) in entries" :key="entry.userId">
        <TableCell class="text-right text-muted-foreground tabular-nums">{{ index + 1 }}</TableCell>
        <TableCell class="max-w-48 truncate font-medium">
          <RouterLink
            :to="{
              name: 'logs',
              query: entry.login ? { channel, user: entry.login } : { channel, userId: entry.userId },
            }"
            class="hover:underline"
          >
            {{ entry.login ?? entry.userId }}
          </RouterLink>
        </TableCell>
        <TableCell v-for="window in WINDOWS" :key="window" class="text-center">
          <Tooltip>
            <TooltipTrigger as-child>
              <span class="inline-flex flex-col items-center gap-0.5">
                <TierBadge v-if="entry.windows[window].tier" :tier="entry.windows[window].tier!" />
                <span class="text-xs text-muted-foreground tabular-nums">
                  {{ n(entry.windows[window].active) }}
                </span>
              </span>
            </TooltipTrigger>
            <TooltipContent>
              {{
                entry.windows[window].rank
                  ? t('tiers.windowTitle', {
                      window,
                      active: n(entry.windows[window].active),
                      rank: entry.windows[window].rank,
                    })
                  : t('tiers.windowUnranked', { window, active: n(entry.windows[window].active) })
              }}
            </TooltipContent>
          </Tooltip>
        </TableCell>
        <TableCell class="text-right font-semibold tabular-nums">{{ n(entry.tierScore) }}</TableCell>
        <TableCell class="hidden text-right tabular-nums sm:table-cell">{{ n(entry.messages) }}</TableCell>
        <TableCell class="hidden text-right text-muted-foreground tabular-nums md:table-cell">
          {{ n(entry.uniqueMessages) }}
        </TableCell>
      </TableRow>
    </TableBody>
  </Table>
</template>
