<script setup lang="ts">
import { ChartNoAxesColumn, MessagesSquare } from '@lucide/vue'
import { useI18n } from 'vue-i18n'
import { useChannels } from '@/api/queries'
import ErrorAlert from '@/components/ErrorAlert.vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'

const { t } = useI18n()
const channels = useChannels()
</script>

<template>
  <section class="py-8 sm:py-12">
    <h1 class="text-3xl font-bold tracking-tight sm:text-4xl">{{ t('home.title') }}</h1>
    <p class="mt-3 max-w-2xl text-muted-foreground">{{ t('home.subtitle') }}</p>
  </section>

  <section>
    <h2 class="mb-4 text-sm font-medium tracking-wide text-muted-foreground uppercase">
      {{ t('home.channels') }}
    </h2>

    <ErrorAlert v-if="channels.error.value" :error="channels.error.value" @retry="channels.refetch()" />

    <div v-else-if="channels.isPending.value" class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      <Skeleton v-for="index in 6" :key="index" class="h-20" />
    </div>

    <p v-else-if="!channels.data.value?.length" class="text-muted-foreground">
      {{ t('common.notLogged') }}
    </p>

    <div v-else class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      <Card v-for="channel in channels.data.value" :key="channel.id" class="py-4">
        <CardContent class="flex items-center justify-between gap-2 px-4">
          <span class="truncate font-medium">{{ channel.login }}</span>
          <div class="flex shrink-0 gap-1">
            <Button as-child variant="secondary" size="sm">
              <RouterLink :to="{ name: 'logs', query: { channel: channel.login } }">
                <MessagesSquare class="size-4" />
                {{ t('home.openLogs') }}
              </RouterLink>
            </Button>
            <Button as-child variant="secondary" size="sm">
              <RouterLink :to="{ name: 'tiers', query: { channel: channel.login } }">
                <ChartNoAxesColumn class="size-4" />
                {{ t('home.openTiers') }}
              </RouterLink>
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  </section>
</template>
