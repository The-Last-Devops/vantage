import { createRouter, createWebHistory } from 'vue-router'
import { useAuth } from '../stores/auth'

const routes = [
  { path: '/login', name: 'login', component: () => import('../pages/Login.vue'), meta: { public: true } },
  { path: '/', name: 'systems', component: () => import('../pages/Systems.vue') },
  { path: '/system/:id', name: 'system', component: () => import('../pages/SystemDetail.vue') },
  { path: '/namespaces', name: 'namespaces', component: () => import('../pages/Namespaces.vue') },
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
