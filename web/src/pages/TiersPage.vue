<script setup lang="ts">
import { ChevronLeft, ChevronRight } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import type { TierMode } from '@/api/client'
import { useTiers, useUser } from '@/api/queries'
import ChannelSelect from '@/components/ChannelSelect.vue'
import ErrorAlert from '@/components/ErrorAlert.vue'
import TierTable from '@/components/tiers/TierTable.vue'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import {
  convertPeriod,
  currentPeriod,
  type Granularity,
  granularityOf,
  shiftPeriod,
} from '@/lib/periods'

/** Tier periods are calendar periods in Moscow time. */
const TIMEZONE = 'Europe/Moscow'
const MODES: TierMode[] = ['all', 'online', 'offline']
const GRANULARITIES: Granularity[] = ['day', 'month', 'year']

const { t, n } = useI18n()
const route = useRoute()
const router = useRouter()

function queryString(name: string): string | undefined {
  const value = route.query[name]
  return typeof value === 'string' && value !== '' ? value : undefined
}

function setQuery(changes: Record<string, string | undefined>) {
  const query = { ...route.query, ...changes }
  for (const [key, value] of Object.entries(query)) if (value === undefined) delete query[key]
  router.replace({ query })
}

const channelLogin = computed(() => queryString('channel'))
const period = computed(() => {
  const value = queryString('period')
  return value && granularityOf(value) ? value : currentPeriod('month', TIMEZONE)
})
const granularity = computed(() => granularityOf(period.value)!)
const mode = computed<TierMode>(() => {
  const value = queryString('mode') as TierMode | undefined
  return value && MODES.includes(value) ? value : 'all'
})

const channel = useUser(() => (channelLogin.value ? { login: channelLogin.value } : undefined))
const tiers = useTiers(() =>
  channel.data.value
    ? { channelId: channel.data.value.id, period: period.value, mode: mode.value }
    : undefined,
)

function selectGranularity(value: unknown) {
  if (typeof value === 'string' && value !== granularity.value) {
    setQuery({ period: convertPeriod(period.value, value as Granularity) })
  }
}

function selectMode(value: unknown) {
  if (typeof value === 'string') setQuery({ mode: value === 'all' ? undefined : value })
}

/** The native date, month or number input for the current granularity. */
const periodInput = computed(() => {
  if (granularity.value === 'day') return { type: 'date', value: period.value }
  if (granularity.value === 'month') return { type: 'month', value: period.value }
  return { type: 'number', value: period.value }
})

function inputPeriod(value: string | number) {
  const text = String(value)
  if (granularityOf(text) === granularity.value) setQuery({ period: text })
}

const error = computed(() => channel.error.value ?? tiers.error.value)
</script>

<template>
  <div class="space-y-4">
    <div>
      <h1 class="text-2xl font-bold tracking-tight">{{ t('tiers.title') }}</h1>
      <p class="mt-1 max-w-3xl text-sm text-muted-foreground">{{ t('tiers.description') }}</p>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <ChannelSelect
        :model-value="channelLogin"
        @update:model-value="(login) => setQuery({ channel: login })"
      />

      <ToggleGroup
        type="single"
        variant="outline"
        :model-value="granularity"
        @update:model-value="selectGranularity"
      >
        <ToggleGroupItem v-for="value in GRANULARITIES" :key="value" :value="value">
          {{ t(`tiers.${value}`) }}
        </ToggleGroupItem>
      </ToggleGroup>

      <div class="flex items-center gap-1">
        <Button
          variant="outline"
          size="icon"
          :aria-label="t('common.previous')"
          @click="setQuery({ period: shiftPeriod(period, -1) })"
        >
          <ChevronLeft class="size-4" />
        </Button>
        <Input
          :type="periodInput.type"
          :model-value="periodInput.value"
          :min="periodInput.type === 'number' ? 2015 : undefined"
          class="w-40"
          @update:model-value="inputPeriod"
        />
        <Button
          variant="outline"
          size="icon"
          :aria-label="t('common.next')"
          @click="setQuery({ period: shiftPeriod(period, 1) })"
        >
          <ChevronRight class="size-4" />
        </Button>
      </div>

      <ToggleGroup type="single" variant="outline" :model-value="mode" @update:model-value="selectMode">
        <ToggleGroupItem v-for="value in MODES" :key="value" :value="value">
          {{ t(`tiers.mode.${value}`) }}
        </ToggleGroupItem>
      </ToggleGroup>
    </div>

    <p v-if="!channelLogin" class="py-16 text-center text-muted-foreground">
      {{ t('tiers.pickChannel') }}
    </p>

    <ErrorAlert v-else-if="error" :error="error" @retry="channel.refetch(); tiers.refetch()" />

    <div v-else-if="!tiers.data.value" class="space-y-2">
      <Skeleton v-for="index in 10" :key="index" class="h-10" />
    </div>

    <template v-else>
      <p class="text-sm text-muted-foreground">
        {{ t('tiers.users', { count: n(tiers.data.value.totalUsers) }) }} ·
        {{ t('tiers.totalMessages', { count: n(tiers.data.value.totalMessages) }) }}
      </p>
      <p v-if="!tiers.data.value.entries.length" class="py-16 text-center text-muted-foreground">
        {{ t('tiers.empty') }}
      </p>
      <Card v-else class="overflow-x-auto py-0">
        <TierTable :entries="tiers.data.value.entries" :channel="channelLogin" />
      </Card>
    </template>
  </div>
</template>
