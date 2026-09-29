<script setup lang="ts">
import { CircleAlert } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { ApiError } from '@/api/client'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'

const props = defineProps<{ error: Error }>()
const emit = defineEmits<{ retry: [] }>()
const { t } = useI18n()

const message = computed(() =>
  props.error instanceof ApiError ? t(`errors.${props.error.code}`) : t('errors.internal_error'),
)
const canRetry = computed(
  () => !(props.error instanceof ApiError) || props.error.problem.status >= 500,
)
</script>

<template>
  <Alert variant="destructive">
    <CircleAlert />
    <AlertDescription class="flex flex-wrap items-center justify-between gap-2">
      <span>{{ message }}</span>
      <Button v-if="canRetry" variant="outline" size="sm" @click="emit('retry')">
        {{ t('common.retry') }}
      </Button>
    </AlertDescription>
  </Alert>
</template>
