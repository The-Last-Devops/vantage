import { createRouter, createWebHistory } from 'vue-router'
import { useAuth } from '../stores/auth'

const routes = [
  { path: '/login', name: 'login', component: () => import('../pages/Login.vue'), meta: { public: true } },
  { path: '/', name: 'systems', component: () => import('../pages/Systems.vue') },
  { path: '/attention', name: 'attention', component: () => import('../pages/Systems.vue') },
  { path: '/system/:id', name: 'system', component: () => import('../pages/SystemDetail.vue') },
  { path: '/namespaces', name: 'namespaces', component: () => import('../pages/Namespaces.vue') },
  { path: '/members', name: 'members', component: () => import('../pages/Members.vue') },
  { path: '/monitors', name: 'monitors', component: () => import('../pages/Monitors.vue') },
  { path: '/notifications', name: 'notifications', component: () => import('../pages/Notifications.vue') },
  { path: '/alerts', name: 'alerts', component: () => import('../pages/Alerts.vue') },
  { path: '/data', name: 'data', component: () => import('../pages/DataRetention.vue') },
  { path: '/audit', name: 'audit', component: () => import('../pages/Audit.vue') },
  { path: '/about', name: 'about', component: () => import('../pages/About.vue') },
]

const router = createRouter({
  history: createWebHistory('/'),
  routes,
})

router.beforeEach(async (to) => {
  const auth = useAuth()
  if (!auth.ready) await auth.bootstrap()
  if (!to.meta.public && !auth.isAuthed) return { name: 'login', query: { next: to.fullPath } }
  if (to.name === 'login' && auth.isAuthed) return { name: 'systems' }
})

export default router
