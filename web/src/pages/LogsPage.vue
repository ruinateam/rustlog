<script setup lang="ts">
import { ChevronLeft, ChevronRight, Dices, Search, Settings2, X } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import {
  type UserRef,
  useChatBadges,
  useLogDates,
  useLogMonths,
  useMessages,
  useRandomMessage,
  useUser,
} from '@/api/queries'
import ChannelSelect from '@/components/ChannelSelect.vue'
import ErrorAlert from '@/components/ErrorAlert.vue'
import MessageLine from '@/components/logs/MessageLine.vue'
import MessageList from '@/components/logs/MessageList.vue'
import StatsPanel from '@/components/logs/StatsPanel.vue'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import { useSettings } from '@/composables/useSettings'
import { useThirdPartyEmotes } from '@/composables/useThirdPartyEmotes'
import { dayRange, monthRange } from '@/lib/periods'

const { t, d, n } = useI18n()
const route = useRoute()
const router = useRouter()
const settings = useSettings()

function queryString(name: string): string | undefined {
  const value = route.query[name]
  return typeof value === 'string' && value !== '' ? value : undefined
}

/** Replaces some query parameters; `undefined` removes one. */
function setQuery(changes: Record<string, string | undefined>, push = true) {
  const query = { ...route.query, ...changes }
  for (const [key, value] of Object.entries(query)) if (value === undefined) delete query[key]
  const location = { query }
  return push ? router.push(location) : router.replace(location)
}

const channelLogin = computed(() => queryString('channel'))
const userRef = computed<UserRef | undefined>(() => {
  const login = queryString('user')
  const id = queryString('userId')
  return login ? { login } : id ? { id } : undefined
})

const channel = useUser(() => (channelLogin.value ? { login: channelLogin.value } : undefined))
const user = useUser(userRef)
const channelId = computed(() => channel.data.value?.id)
const userId = computed(() => (userRef.value ? user.data.value?.id : undefined))

// A user picked by id gets the login in the URL once it is known.
watch(user.data, (found) => {
  if (found && queryString('userId') === found.id) {
    setQuery({ userId: undefined, user: found.login }, false)
  }
})

// Days of the channel, or months of the user; newest first.
const dates = useLogDates(() => (userRef.value ? undefined : channelId.value))
const months = useLogMonths(channelId, userId)
const periods = computed(() => (userRef.value ? months.data.value : dates.data.value) ?? [])
const periodParam = computed(() => (userRef.value ? 'month' : 'date'))
const period = computed(() => queryString(periodParam.value) ?? periods.value[0])
const periodIndex = computed(() => (period.value ? periods.value.indexOf(period.value) : -1))

function selectPeriod(value: unknown) {
  if (typeof value === 'string') setQuery({ [periodParam.value]: value })
}

function formatPeriod(value: string) {
  return userRef.value
    ? d(new Date(`${value}-01T00:00:00Z`), { year: 'numeric', month: 'long', timeZone: 'UTC' })
    : d(new Date(`${value}T00:00:00Z`), { dateStyle: 'medium', timeZone: 'UTC' })
}

const range = computed(() => {
  if (!period.value) return undefined
  return userRef.value ? monthRange(period.value) : dayRange(period.value)
})

const messages = useMessages(channelId, userId, range)
const badges = useChatBadges(channelId)
const badgesByKey = computed(
  () => new Map((badges.data.value ?? []).map((badge) => [`${badge.setId}/${badge.version}`, badge])),
)
const emotes = useThirdPartyEmotes(channelId)

const filter = ref('')
const shownMessages = computed(() => {
  const needle = filter.value.trim().toLowerCase()
  const all = messages.data.value ?? []
  const matching = needle
    ? all.filter(
        (message) =>
          message.text.toLowerCase().includes(needle) ||
          message.displayName.toLowerCase().includes(needle),
      )
    : all
  return settings.value.newestFirst ? [...matching].reverse() : matching
})

const userDraft = ref('')
watch(
  () => user.data.value?.login ?? queryString('user') ?? '',
  (login) => (userDraft.value = login),
  { immediate: true },
)

function applyUser() {
  const login = userDraft.value.trim().toLowerCase()
  setQuery({ user: login || undefined, userId: undefined, month: undefined, date: undefined })
}

function selectUser(id: string) {
  setQuery({ user: undefined, userId: id, month: undefined, date: undefined })
}

function selectChannel(login: string | undefined) {
  setQuery({ channel: login, user: undefined, userId: undefined, date: undefined, month: undefined })
}

const random = useRandomMessage()
function pickRandom() {
  if (channelId.value) random.mutate({ channelId: channelId.value, userId: userId.value })
}
watch([channelId, userId], () => random.reset())

const lookupError = computed(() => channel.error.value ?? user.error.value)
const listError = computed(() => (userRef.value ? months.error.value : dates.error.value) ?? messages.error.value)
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-end gap-2">
      <ChannelSelect :model-value="channelLogin" @update:model-value="selectChannel" />

      <form class="flex items-center gap-1" @submit.prevent="applyUser">
        <div class="relative">
          <Input
            v-model="userDraft"
            :placeholder="t('logs.userPlaceholder')"
            :aria-label="t('common.user')"
            class="w-48 pr-8"
            :disabled="!channelLogin"
          />
          <button
            v-if="userDraft"
            type="button"
            class="absolute top-1/2 right-2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            :aria-label="t('common.anyUser')"
            @click="userDraft = ''; applyUser()"
          >
            <X class="size-4" />
          </button>
        </div>
      </form>

      <div v-if="channelLogin" class="flex items-center gap-1">
        <Button
          variant="outline"
          size="icon"
          :aria-label="t('common.previous')"
          :disabled="periodIndex < 0 || periodIndex >= periods.length - 1"
          @click="selectPeriod(periods[periodIndex + 1])"
        >
          <ChevronLeft class="size-4" />
        </Button>
        <Select :model-value="period" @update:model-value="selectPeriod">
          <SelectTrigger class="w-44" :aria-label="userRef ? t('logs.month') : t('logs.date')">
            <SelectValue :placeholder="t('logs.noDates')">
              {{ period ? formatPeriod(period) : t('logs.noDates') }}
            </SelectValue>
          </SelectTrigger>
          <SelectContent class="max-h-80">
            <SelectItem v-for="value in periods" :key="value" :value="value">
              {{ formatPeriod(value) }}
            </SelectItem>
          </SelectContent>
        </Select>
        <Button
          variant="outline"
          size="icon"
          :aria-label="t('common.next')"
          :disabled="periodIndex <= 0"
          @click="selectPeriod(periods[periodIndex - 1])"
        >
          <ChevronRight class="size-4" />
        </Button>
      </div>

      <div v-if="channelId" class="ml-auto flex items-center gap-1">
        <Button variant="outline" :disabled="random.isPending.value" @click="pickRandom">
          <Dices class="size-4" />
          <span class="hidden sm:inline">{{ t('logs.random') }}</span>
        </Button>
        <Popover>
          <PopoverTrigger as-child>
            <Button variant="outline" size="icon" :aria-label="t('logs.settings')">
              <Settings2 class="size-4" />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="end" class="w-56 space-y-3">
            <p class="text-sm font-medium">{{ t('logs.settings') }}</p>
            <div
              v-for="key in ['showTimestamps', 'showBadges', 'showEmotes', 'newestFirst'] as const"
              :key="key"
              class="flex items-center justify-between"
            >
              <Label :for="key">{{ t(`logs.${key}`) }}</Label>
              <Switch :id="key" v-model="settings[key]" />
            </div>
          </PopoverContent>
        </Popover>
      </div>
    </div>

    <p v-if="!channelLogin" class="py-16 text-center text-muted-foreground">
      {{ t('logs.pickChannel') }}
    </p>

    <ErrorAlert v-else-if="lookupError" :error="lookupError" @retry="channel.refetch(); user.refetch()" />

    <div v-else class="grid gap-4 lg:grid-cols-[1fr_16rem]">
      <div class="min-w-0 space-y-3">
        <Card v-if="random.data.value" class="border-primary/40 py-2">
          <MessageLine
            :message="random.data.value"
            :badges="badgesByKey"
            :emotes="emotes"
            :settings="{ ...settings, showTimestamps: true }"
            @select-user="selectUser"
          />
        </Card>
        <ErrorAlert v-if="random.error.value" :error="random.error.value" @retry="pickRandom" />

        <Card class="gap-0 overflow-hidden py-0">
          <div class="flex items-center gap-2 border-b px-3 py-2">
            <Search class="size-4 shrink-0 text-muted-foreground" />
            <input
              v-model="filter"
              type="search"
              :placeholder="t('logs.search')"
              class="w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
            />
            <span v-if="messages.data.value" class="shrink-0 text-xs text-muted-foreground tabular-nums">
              {{
                t('logs.shown', {
                  shown: n(shownMessages.length),
                  total: n(messages.data.value.length),
                })
              }}
            </span>
          </div>

          <div v-if="listError" class="p-3">
            <ErrorAlert :error="listError" @retry="dates.refetch(); months.refetch(); messages.refetch()" />
          </div>
          <div v-else-if="messages.isLoading.value || channel.isPending.value" class="space-y-2 p-3">
            <Skeleton v-for="index in 12" :key="index" class="h-5" :style="{ width: `${40 + ((index * 37) % 55)}%` }" />
          </div>
          <p
            v-else-if="!shownMessages.length"
            class="py-16 text-center text-sm text-muted-foreground"
          >
            {{ periods.length ? (filter ? t('logs.noMatches') : t('logs.empty')) : t('logs.noDates') }}
          </p>
          <MessageList
            v-else
            :messages="shownMessages"
            :badges="badgesByKey"
            :emotes="emotes"
            :settings="settings"
            @select-user="selectUser"
          />
        </Card>
      </div>

      <aside v-if="channelId">
        <StatsPanel :channel-id="channelId" :user-id="userId" @select-user="selectUser" />
      </aside>
    </div>
  </div>
</template>
