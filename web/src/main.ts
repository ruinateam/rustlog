import '@fontsource-variable/inter'
import '@fontsource-variable/jetbrains-mono'
import './style.css'

import { VueQueryPlugin } from '@tanstack/vue-query'
import { createApp } from 'vue'
import App from './App.vue'
import { i18n } from './i18n'
import { router } from './router'

createApp(App)
  .use(router)
  .use(i18n)
  .use(VueQueryPlugin, {
    queryClientConfig: { defaultOptions: { queries: { refetchOnWindowFocus: false } } },
  })
  .mount('#app')
