import { defineStore } from 'pinia'

// Theme + workspace filter, shared across the shell and pages.
export const useUi = defineStore('ui', {
  state: () => ({
    light: localStorage.getItem('vantage-theme') === 'light',
    // Set of selected workspace names; empty Set with `allWs` flag = show all.
    selectedWs: new Set(JSON.parse(localStorage.getItem('vantage-ws') || '[]')),
    wsTouched: localStorage.getItem('vantage-ws') != null,
  }),
  actions: {
    applyTheme() {
      document.documentElement.classList.toggle('light', this.light)
    },
    toggleTheme() {
      this.light = !this.light
      localStorage.setItem('vantage-theme', this.light ? 'light' : 'dark')
      this.applyTheme()
    },
    // workspace filter: if untouched, treat as "all"
    inWs(ws, allNames) {
      if (!this.wsTouched) return true
      if (this.selectedWs.size === 0) return false
      return this.selectedWs.has(ws)
    },
    toggleWs(name, allNames) {
      if (!this.wsTouched) {
        // first interaction starts from "all selected"
        this.selectedWs = new Set(allNames)
        this.wsTouched = true
      }
      this.selectedWs.has(name) ? this.selectedWs.delete(name) : this.selectedWs.add(name)
      this._persistWs(allNames)
    },
    toggleAllWs(allNames) {
      const all = this.wsTouched ? this.selectedWs.size === allNames.length : true
      this.selectedWs = all ? new Set() : new Set(allNames)
      this.wsTouched = true
      this._persistWs(allNames)
    },
    _persistWs() {
      localStorage.setItem('vantage-ws', JSON.stringify([...this.selectedWs]))
    },
  },
})
