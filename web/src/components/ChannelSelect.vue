<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { useChannels } from '@/api/queries'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

/** Login of the chosen logged channel. */
const channel = defineModel<string | undefined>()
const { t } = useI18n()
const channels = useChannels()
</script>

<template>
  <Select v-model="channel">
    <SelectTrigger class="w-48" :aria-label="t('common.channel')">
      <SelectValue :placeholder="t('common.channel')" />
    </SelectTrigger>
    <SelectContent>
      <SelectItem v-for="item in channels.data.value" :key="item.id" :value="item.login">
        {{ item.login }}
      </SelectItem>
    </SelectContent>
  </Select>
</template>
