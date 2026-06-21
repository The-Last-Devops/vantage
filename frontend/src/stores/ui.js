import { defineStore } from 'pinia'

// Theme + namespace filter, shared across the shell and pages.
export const useUi = defineStore('ui', {
  state: () => ({
    light: localStorage.getItem('lm-theme') === 'light',
    // Set of selected namespace names; empty Set with `allNs` flag = show all.
    selectedNs: new Set(JSON.parse(localStorage.getItem('lm-ns') || '[]')),
    nsTouched: localStorage.getItem('lm-ns') != null,
  }),
  actions: {
    applyTheme() {
      document.documentElement.classList.toggle('light', this.light)
    },
    toggleTheme() {
      this.light = !this.light
      localStorage.setItem('lm-theme', this.light ? 'light' : 'dark')
      this.applyTheme()
    },
    // namespace filter: if untouched, treat as "all"
    inNs(ns, allNames) {
      if (!this.nsTouched) return true
      if (this.selectedNs.size === 0) return false
      return this.selectedNs.has(ns)
    },
    toggleNs(name, allNames) {
      if (!this.nsTouched) {
        // first interaction starts from "all selected"
        this.selectedNs = new Set(allNames)
        this.nsTouched = true
      }
      this.selectedNs.has(name) ? this.selectedNs.delete(name) : this.selectedNs.add(name)
      this._persistNs(allNames)
    },
    toggleAllNs(allNames) {
      const all = this.nsTouched ? this.selectedNs.size === allNames.length : true
      this.selectedNs = all ? new Set() : new Set(allNames)
      this.nsTouched = true
      this._persistNs(allNames)
    },
    _persistNs() {
      localStorage.setItem('lm-ns', JSON.stringify([...this.selectedNs]))
    },
  },
})
