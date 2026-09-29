<script setup lang="ts">
import { Check, Copy } from '@lucide/vue'
import { useClipboard } from '@vueuse/core'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useOptOutCode } from '@/api/queries'
import ErrorAlert from '@/components/ErrorAlert.vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

const { t, d } = useI18n()
const optOut = useOptOutCode()
const { copy, copied } = useClipboard()

const command = computed(() =>
  optOut.data.value ? `!rustlog optout ${optOut.data.value.code}` : undefined,
)
</script>

<template>
  <Card class="mx-auto max-w-lg">
    <CardHeader>
      <CardTitle>{{ t('optOut.title') }}</CardTitle>
      <CardDescription>{{ t('optOut.description') }}</CardDescription>
    </CardHeader>
    <CardContent class="space-y-4">
      <ErrorAlert v-if="optOut.error.value" :error="optOut.error.value" @retry="optOut.mutate()" />

      <div v-if="command" class="space-y-2">
        <p class="text-sm font-medium">{{ t('optOut.command') }}</p>
        <div class="flex items-center gap-2">
          <code class="flex-1 rounded-md bg-muted px-3 py-2 font-mono text-sm">{{ command }}</code>
          <Button variant="outline" size="icon" :aria-label="t('optOut.copy')" @click="copy(command)">
            <Check v-if="copied" class="size-4" />
            <Copy v-else class="size-4" />
          </Button>
        </div>
        <p class="text-xs text-muted-foreground">
          {{ t('optOut.expires', { time: d(new Date(optOut.data.value!.expiresAt), { timeStyle: 'medium' }) }) }}
        </p>
      </div>

      <Button :disabled="optOut.isPending.value" @click="optOut.mutate()">
        {{ t('optOut.getCode') }}
      </Button>
    </CardContent>
  </Card>
</template>
