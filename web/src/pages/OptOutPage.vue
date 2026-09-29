<script setup lang="ts">
import { Check, Copy } from '@lucide/vue'
import { useClipboard } from '@vueuse/core'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useOptOutCode } from '@/api/queries'
import ErrorAlert from '@/components/ErrorAlert.vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

type Audience = 'viewer' | 'streamer'

const { t, d } = useI18n()
const optOut = useOptOutCode()
const { copy, copied } = useClipboard()
const copiedCommand = ref<string>()

/** The chat commands of each audience: to opt out, and to opt back in. */
const COMMANDS: Record<Audience, { command: string; label: string }[]> = {
  viewer: [
    { command: 'optout', label: 'optOut.viewer.optOut' },
    { command: 'optin', label: 'optOut.viewer.optIn' },
  ],
  streamer: [
    { command: 'optout-channel', label: 'optOut.streamer.optOut' },
    { command: 'optin-channel', label: 'optOut.streamer.optIn' },
  ],
}

const code = computed(() => optOut.data.value?.code ?? t('optOut.codePlaceholder'))

function commandText(command: string) {
  return `!rustlog ${command} ${code.value}`
}

function copyCommand(command: string) {
  copiedCommand.value = command
  copy(commandText(command))
}
</script>

<template>
  <div class="mx-auto max-w-2xl space-y-4">
    <div>
      <h1 class="text-2xl font-bold tracking-tight">{{ t('optOut.title') }}</h1>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('optOut.description') }}</p>
    </div>

    <Card class="gap-4">
      <CardContent class="flex flex-wrap items-center gap-3">
        <Button :disabled="optOut.isPending.value" @click="optOut.mutate()">
          {{ optOut.data.value ? t('optOut.newCode') : t('optOut.getCode') }}
        </Button>
        <template v-if="optOut.data.value">
          <code class="rounded-md bg-muted px-3 py-1.5 font-mono text-lg tracking-widest">
            {{ optOut.data.value.code }}
          </code>
          <span class="text-sm text-muted-foreground">
            {{ t('optOut.expires', { time: d(new Date(optOut.data.value.expiresAt), { timeStyle: 'medium' }) }) }}
          </span>
        </template>
      </CardContent>
    </Card>

    <ErrorAlert v-if="optOut.error.value" :error="optOut.error.value" @retry="optOut.mutate()" />

    <Tabs default-value="viewer">
      <TabsList class="w-full">
        <TabsTrigger value="viewer">{{ t('optOut.viewer.tab') }}</TabsTrigger>
        <TabsTrigger value="streamer">{{ t('optOut.streamer.tab') }}</TabsTrigger>
      </TabsList>

      <TabsContent v-for="audience in ['viewer', 'streamer'] as const" :key="audience" :value="audience">
        <Card>
          <CardHeader>
            <CardTitle class="text-base">{{ t(`optOut.${audience}.title`) }}</CardTitle>
            <CardDescription>{{ t(`optOut.${audience}.description`) }}</CardDescription>
          </CardHeader>
          <CardContent class="space-y-4">
            <div v-for="item in COMMANDS[audience]" :key="item.command" class="space-y-1.5">
              <p class="text-sm font-medium">{{ t(item.label) }}</p>
              <div class="flex items-center gap-2">
                <code class="flex-1 rounded-md bg-muted px-3 py-2 font-mono text-sm">
                  {{ commandText(item.command) }}
                </code>
                <Button
                  variant="outline"
                  size="icon"
                  :disabled="!optOut.data.value"
                  :aria-label="t('optOut.copy')"
                  @click="copyCommand(item.command)"
                >
                  <Check v-if="copied && copiedCommand === item.command" class="size-4" />
                  <Copy v-else class="size-4" />
                </Button>
              </div>
            </div>
            <p class="text-xs text-muted-foreground">{{ t(`optOut.${audience}.note`) }}</p>
          </CardContent>
        </Card>
      </TabsContent>
    </Tabs>
  </div>
</template>
