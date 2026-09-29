<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { useChannelStats, useNameHistory, useUserStats } from '@/api/queries'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'

const props = defineProps<{ channelId: string; userId?: string }>()
const emit = defineEmits<{ selectUser: [userId: string] }>()
const { t, n, d } = useI18n()

const channelStats = useChannelStats(() => (props.userId ? undefined : props.channelId))
const userStats = useUserStats(
  () => props.channelId,
  () => props.userId,
)
const names = useNameHistory(() => props.userId)
</script>

<template>
  <Card class="gap-3 py-4">
    <CardHeader class="px-4">
      <CardTitle class="text-sm">{{ t('logs.stats') }}</CardTitle>
    </CardHeader>
    <CardContent class="space-y-4 px-4 text-sm">
      <template v-if="userId">
        <div>
          <p class="text-muted-foreground">{{ t('logs.messageCount') }}</p>
          <Skeleton v-if="userStats.isPending.value" class="mt-1 h-6 w-16" />
          <p v-else class="text-lg font-semibold">
            {{ n(userStats.data.value?.messageCount ?? 0) }}
          </p>
        </div>
        <div v-if="names.data.value?.length">
          <p class="mb-1 text-muted-foreground">{{ t('logs.previousNames') }}</p>
          <ul class="space-y-1">
            <li v-for="name in names.data.value" :key="name.login">
              <span class="font-medium">{{ name.login }}</span>
              <span class="ml-1 text-xs text-muted-foreground">
                {{ t('logs.firstSeen', { date: d(new Date(name.firstSeenAt), { dateStyle: 'medium' }) }) }}
              </span>
            </li>
          </ul>
        </div>
      </template>

      <template v-else>
        <div>
          <p class="text-muted-foreground">{{ t('logs.messageCount') }}</p>
          <Skeleton v-if="channelStats.isPending.value" class="mt-1 h-6 w-20" />
          <p v-else class="text-lg font-semibold">
            {{ n(channelStats.data.value?.messageCount ?? 0) }}
          </p>
        </div>
        <div v-if="channelStats.data.value?.topChatters.length">
          <p class="mb-1 text-muted-foreground">{{ t('logs.topChatters') }}</p>
          <ol class="space-y-0.5">
            <li
              v-for="chatter in channelStats.data.value.topChatters.slice(0, 10)"
              :key="chatter.userId"
              class="flex justify-between gap-2"
            >
              <button
                type="button"
                class="truncate hover:underline"
                :class="{ 'text-muted-foreground': !chatter.login }"
                @click="emit('selectUser', chatter.userId)"
              >
                {{ chatter.login ?? chatter.userId }}
              </button>
              <span class="text-muted-foreground tabular-nums">{{ n(chatter.messageCount) }}</span>
            </li>
          </ol>
        </div>
      </template>
    </CardContent>
  </Card>
</template>
