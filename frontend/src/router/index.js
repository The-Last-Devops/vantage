import { createRouter, createWebHistory } from 'vue-router'
import { useAuth } from '../stores/auth'

// Pages are imported eagerly (not lazy `() => import()`). Lazy routes fetch a
// JS chunk on first navigation, and the router renders nothing until it lands —
// that gap is the brief blank/black flash when switching pages. Eager imports
// make every navigation synchronous, so the new page paints in one frame. The
// whole app is small enough (single-binary internal tool) that one bundle is fine.
import Login from '../pages/Login.vue'
import Overview from '../pages/Overview.vue'
import FleetOverview from '../pages/FleetOverview.vue'
import FleetMetrics from '../pages/FleetMetrics.vue'
import Systems from '../pages/Systems.vue'
import SystemDetail from '../pages/SystemDetail.vue'
import Console from '../pages/Console.vue'
import SecuritySettings from '../pages/SecuritySettings.vue'
import Workspaces from '../pages/Workspaces.vue'
import WorkspaceDetail from '../pages/WorkspaceDetail.vue'
import Members from '../pages/Members.vue'
import Monitors from '../pages/Monitors.vue'
import MonitorDetail from '../pages/MonitorDetail.vue'
import MonitorEdit from '../pages/MonitorEdit.vue'
import Notifications from '../pages/Notifications.vue'
import ChannelDetail from '../pages/ChannelDetail.vue'
import Events from '../pages/Events.vue'
import Alerts from '../pages/Alerts.vue'
import AlertEdit from '../pages/AlertEdit.vue'
import DataRetention from '../pages/DataRetention.vue'
import Backup from '../pages/Backup.vue'
import Audit from '../pages/Audit.vue'
import ApiTokens from '../pages/ApiTokens.vue'
import SshKeys from '../pages/SshKeys.vue'
import About from '../pages/About.vue'

// Every route carries meta.title — AppHeader falls back to it so the header is never
// blank. Pages may still pass an explicit `title` (dynamic) or `breadcrumb` to override.
const routes = [
  { path: '/login', name: 'login', component: Login, meta: { public: true, title: 'Sign in' } },
  { path: '/', name: 'systems', component: Systems, meta: { title: 'Infrastructure' } },
  { path: '/overview', name: 'overview', component: Overview, meta: { title: 'Overview' } },
  { path: '/fleet', name: 'fleet', component: FleetOverview, meta: { title: 'Fleet' } },
  { path: '/metrics', name: 'metrics', component: FleetMetrics, meta: { title: 'Metrics' } },
  { path: '/attention', name: 'attention', component: Systems, meta: { title: 'Issues' } },
  { path: '/system/:id', name: 'system', component: SystemDetail, meta: { title: 'Host' } },
  { path: '/systems/:id/console', name: 'console', component: Console, meta: { title: 'Console' } },
  { path: '/workspaces', name: 'workspaces', component: Workspaces, meta: { title: 'Workspaces' } },
  { path: '/workspace/:id', name: 'workspace', component: WorkspaceDetail, meta: { title: 'Workspace' } },
  { path: '/members', name: 'members', component: Members, meta: { title: 'Members' } },
  { path: '/settings/security', name: 'security', component: SecuritySettings, meta: { title: 'Security' } },
  { path: '/monitors', name: 'monitors', component: Monitors, meta: { title: 'Services' } },
  { path: '/monitor/new', name: 'monitor-new', component: MonitorEdit, meta: { title: 'New service' } },
  { path: '/monitor/:id/edit', name: 'monitor-edit', component: MonitorEdit, meta: { title: 'Edit service' } },
  { path: '/monitor/:id', name: 'monitor', component: MonitorDetail, meta: { title: 'Service' } },
  { path: '/notifications', name: 'notifications', component: Notifications, meta: { title: 'Notify channels' } },
  { path: '/channel/:id', name: 'channel', component: ChannelDetail, meta: { title: 'Channel' } },
  { path: '/events', name: 'events', component: Events, meta: { title: 'Events' } },
  { path: '/alerts', name: 'alerts', component: Alerts, meta: { title: 'Alert rules' } },
  { path: '/alerts/new', name: 'alert-new', component: AlertEdit, meta: { title: 'New rule' } },
  { path: '/alerts/:id/edit', name: 'alert-edit', component: AlertEdit, meta: { title: 'Edit rule' } },
  { path: '/data', name: 'data', component: DataRetention, meta: { title: 'Data & retention' } },
  { path: '/backup', name: 'backup', component: Backup, meta: { title: 'Backup & restore' } },
  { path: '/audit', name: 'audit', component: Audit, meta: { title: 'Audit' } },
  { path: '/tokens', name: 'tokens', component: ApiTokens, meta: { title: 'API tokens' } },
  { path: '/ssh-keys', name: 'ssh-keys', component: SshKeys, meta: { title: 'SSH keys' } },
  { path: '/about', name: 'about', component: About, meta: { title: 'About' } },
]

const router = createRouter({
  history: createWebHistory('/'),
  routes,
})

router.beforeEach(async (to, from) => {
  const auth = useAuth()
  if (!auth.ready) await auth.bootstrap()
  if (!to.meta.public && !auth.isAuthed) return { name: 'login', query: { next: to.fullPath } }
  if (to.name === 'login' && auth.isAuthed) return { name: 'systems' }
  // Keep the workspace selection in the URL across navigation — never drop ?ws
  // when moving to another page. (Same-page changes are respected, so clearing
  // the selector to "all workspaces" still works.)
  if (to.path !== from.path && to.query.ws == null && from.query.ws != null) {
    return { path: to.path, query: { ...to.query, ws: from.query.ws }, hash: to.hash }
  }
})

export default router
