import { createRouter, createWebHistory } from 'vue-router'

// Paths have a single segment: the backend answers longer ones with its
// legacy API (`/{channel_id_type}/{channel}` and so on), so the page state
// goes into the query.
export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', name: 'home', component: () => import('./pages/HomePage.vue') },
    { path: '/logs', name: 'logs', component: () => import('./pages/LogsPage.vue') },
    { path: '/tiers', name: 'tiers', component: () => import('./pages/TiersPage.vue') },
    { path: '/opt-out', name: 'opt-out', component: () => import('./pages/OptOutPage.vue') },
    { path: '/:path(.*)*', name: 'not-found', component: () => import('./pages/NotFoundPage.vue') },
  ],
})
