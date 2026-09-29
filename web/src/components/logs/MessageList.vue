<script setup lang="ts">
import { useVirtualizer } from '@tanstack/vue-virtual'
import { computed, ref, watch } from 'vue'
import type { BasicMessage, Schemas } from '@/api/client'
import type { LogSettings } from '@/composables/useSettings'
import type { Emote } from '@/lib/messages'
import MessageLine from './MessageLine.vue'

const props = defineProps<{
  messages: BasicMessage[]
  badges: ReadonlyMap<string, Schemas['ChatBadge']>
  emotes: ReadonlyMap<string, Emote>
  settings: LogSettings
}>()
const emit = defineEmits<{ selectUser: [userId: string] }>()

const scroller = ref<HTMLElement | null>(null)

// Days can have tens of thousands of messages: only the visible ones are
// rendered, measured as they appear since long messages wrap.
const virtualizer = useVirtualizer(
  computed(() => ({
    count: props.messages.length,
    getScrollElement: () => scroller.value,
    estimateSize: () => 28,
    overscan: 20,
    getItemKey: (index: number) => props.messages[index]?.id || index,
  })),
)

const rows = computed(() => virtualizer.value.getVirtualItems())
const totalHeight = computed(() => virtualizer.value.getTotalSize())

function measure(element: unknown) {
  if (element instanceof Element) virtualizer.value.measureElement(element)
}

watch(
  () => props.messages,
  () => virtualizer.value.scrollToIndex(0),
)
</script>

<template>
  <div ref="scroller" class="h-[calc(100dvh-16rem)] min-h-80 overflow-y-auto">
    <div class="relative w-full" :style="{ height: `${totalHeight}px` }">
      <div
        v-for="row in rows"
        :key="row.key as string"
        :ref="measure"
        :data-index="row.index"
        class="absolute top-0 left-0 w-full"
        :style="{ transform: `translateY(${row.start}px)` }"
      >
        <MessageLine
          :message="messages[row.index]!"
          :badges="badges"
          :emotes="emotes"
          :settings="settings"
          @select-user="emit('selectUser', $event)"
        />
      </div>
    </div>
  </div>
</template>
