import { useStorage } from '@vueuse/core'

export interface LogSettings {
  showTimestamps: boolean
  showBadges: boolean
  showEmotes: boolean
  newestFirst: boolean
}

const settings = useStorage<LogSettings>(
  'rustlog:log-settings',
  { showTimestamps: true, showBadges: true, showEmotes: true, newestFirst: false },
  localStorage,
  { mergeDefaults: true },
)

/** How logs are shown; kept in the browser between visits. */
export function useSettings() {
  return settings
}
