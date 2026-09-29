<script setup lang="ts">
import AppHeader from '@/components/AppHeader.vue'
import { TooltipProvider } from '@/components/ui/tooltip'
</script>

<template>
  <TooltipProvider :delay-duration="300">
    <div class="flex min-h-dvh flex-col">
      <AppHeader />
      <main class="mx-auto w-full max-w-6xl flex-1 px-4 py-6 sm:px-6">
        <RouterView />
      </main>
    </div>
  </TooltipProvider>
</template>
