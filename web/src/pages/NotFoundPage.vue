<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Button } from '@/components/ui/button'

const { t } = useI18n()
</script>

<template>
  <div class="flex flex-col items-center gap-4 py-24 text-center">
    <p class="text-6xl font-bold text-muted-foreground">404</p>
    <h1 class="text-xl font-semibold">{{ t('notFound.title') }}</h1>
    <Button as-child>
      <RouterLink to="/">{{ t('notFound.back') }}</RouterLink>
    </Button>
  </div>
</template>
