<script setup lang="ts">
import { Languages, Moon, Sun } from '@lucide/vue'
import { useColorMode } from '@vueuse/core'
import { useI18n } from 'vue-i18n'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { type Locale, locale } from '@/i18n'

const { t } = useI18n()
const colorMode = useColorMode({ emitAuto: true })

const languages: { locale: Locale; name: string }[] = [
  { locale: 'en', name: 'English' },
  { locale: 'ru', name: 'Русский' },
]

function toggleTheme() {
  colorMode.value = document.documentElement.classList.contains('dark') ? 'light' : 'dark'
}

const links = [
  { to: '/logs', label: 'nav.logs' },
  { to: '/tiers', label: 'nav.tiers' },
  { to: '/opt-out', label: 'nav.optOut' },
]
</script>

<template>
  <header class="sticky top-0 z-40 border-b bg-background/80 backdrop-blur">
    <div class="mx-auto flex h-14 max-w-6xl items-center gap-2 px-4 sm:px-6">
      <RouterLink to="/" class="mr-4 flex items-center gap-2 font-semibold">
        <img src="/favicon.svg" alt="" class="size-6" />
        <span>rustlog</span>
      </RouterLink>

      <nav class="flex items-center gap-1 text-sm">
        <RouterLink
          v-for="link in links"
          :key="link.to"
          :to="link.to"
          class="rounded-md px-3 py-1.5 text-muted-foreground transition-colors hover:text-foreground"
          active-class="bg-accent text-foreground"
        >
          {{ t(link.label) }}
        </RouterLink>
        <a
          href="/docs"
          class="rounded-md px-3 py-1.5 text-muted-foreground transition-colors hover:text-foreground"
        >
          {{ t('nav.apiDocs') }}
        </a>
      </nav>

      <div class="ml-auto flex items-center gap-1">
        <DropdownMenu>
          <DropdownMenuTrigger as-child>
            <Button variant="ghost" size="icon" :aria-label="t('common.language')">
              <Languages class="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              v-for="language in languages"
              :key="language.locale"
              :class="{ 'font-semibold': language.locale === locale }"
              @select="locale = language.locale"
            >
              {{ language.name }}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        <Button variant="ghost" size="icon" :aria-label="t('common.theme')" @click="toggleTheme">
          <Sun class="size-4 dark:hidden" />
          <Moon class="hidden size-4 dark:block" />
        </Button>
      </div>
    </div>
  </header>
</template>
