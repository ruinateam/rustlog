import { useStorage } from '@vueuse/core'
import { watch } from 'vue'
import { createI18n } from 'vue-i18n'
import en from './locales/en'
import ru from './locales/ru'

export const locales = ['en', 'ru'] as const
export type Locale = (typeof locales)[number]

function browserLocale(): Locale {
  return navigator.language.toLowerCase().startsWith('ru') ? 'ru' : 'en'
}

/** The chosen language, the browser's until one is picked. */
export const locale = useStorage<Locale>('rustlog:locale', browserLocale())

export const i18n = createI18n({
  legacy: false,
  locale: locale.value,
  fallbackLocale: 'en',
  messages: { en, ru },
})

watch(
  locale,
  (value) => {
    i18n.global.locale.value = value
    document.documentElement.lang = value
  },
  { immediate: true },
)
