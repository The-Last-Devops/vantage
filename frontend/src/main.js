import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import { useUi } from './stores/ui'
import './style.css'

const app = createApp(App)
app.use(createPinia())
useUi().applyTheme() // apply saved theme before first paint
app.use(router)
app.mount('#app')
