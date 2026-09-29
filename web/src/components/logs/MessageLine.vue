<script setup lang="ts">
import { computed } from 'vue'
import type { BasicMessage, Schemas } from '@/api/client'
import type { LogSettings } from '@/composables/useSettings'
import { badgeKeys, type Emote, tokenize } from '@/lib/messages'

const props = defineProps<{
  message: BasicMessage
  badges: ReadonlyMap<string, Schemas['ChatBadge']>
  emotes: ReadonlyMap<string, Emote>
  settings: LogSettings
}>()
const emit = defineEmits<{ selectUser: [userId: string] }>()

const timestamp = computed(() => new Date(props.message.timestamp))
const time = computed(() => timestamp.value.toLocaleTimeString(undefined, { hour12: false }))
const fullTime = computed(() => timestamp.value.toLocaleString())

const userId = computed(() => props.message.tags['user-id'])
const color = computed(() => props.message.tags['color'] || undefined)
/** Notices, subs and the like carry their text in `system-msg`. */
const systemText = computed(() => props.message.tags['system-msg'])

const messageBadges = computed(() =>
  props.settings.showBadges
    ? badgeKeys(props.message.tags['badges']).flatMap((key) => {
        const badge = props.badges.get(key)
        return badge ? [badge] : []
      })
    : [],
)

const tokens = computed(() => {
  const text = systemText.value
    ? props.message.text.replace(`${systemText.value} `, '')
    : props.message.text
  return tokenize(
    text,
    props.settings.showEmotes ? props.message.tags['emotes'] : undefined,
    props.settings.showEmotes ? props.emotes : new Map(),
  )
})
</script>

<template>
  <div class="group px-3 py-1 text-sm leading-6 break-words hover:bg-accent/50">
    <time
      v-if="settings.showTimestamps"
      :datetime="message.timestamp"
      :title="fullTime"
      class="mr-2 font-mono text-xs text-muted-foreground tabular-nums"
    >
      {{ time }}
    </time>
    <span v-if="systemText" class="mr-1 text-muted-foreground italic">{{ systemText }}</span>
    <span v-if="messageBadges.length" class="mr-1 inline-flex gap-0.5 align-[-3px]">
      <img
        v-for="badge in messageBadges"
        :key="`${badge.setId}/${badge.version}`"
        :src="badge.imageUrl1x"
        :srcset="`${badge.imageUrl1x} 1x, ${badge.imageUrl2x} 2x`"
        :alt="badge.title"
        :title="badge.title"
        class="size-[18px]"
        loading="lazy"
      />
    </span>
    <button
      v-if="message.displayName"
      type="button"
      class="font-semibold hover:underline"
      :style="{ color }"
      :disabled="!userId"
      @click="userId && emit('selectUser', userId)"
    >
      {{ message.displayName }}</button
    ><span v-if="message.displayName">: </span>
    <template v-for="(token, index) in tokens" :key="index">
      <img
        v-if="token.kind === 'emote'"
        :src="token.emote.url"
        :srcset="token.emote.srcset"
        :alt="token.emote.code"
        :title="token.emote.code"
        class="mx-0.5 inline h-7 w-auto align-middle"
        loading="lazy"
      />
      <a
        v-else-if="token.kind === 'link'"
        :href="token.url"
        target="_blank"
        rel="noopener noreferrer nofollow"
        class="text-primary underline-offset-2 hover:underline"
        >{{ token.url }}</a
      >
      <span v-else>{{ token.text }}</span>
    </template>
  </div>
</template>
