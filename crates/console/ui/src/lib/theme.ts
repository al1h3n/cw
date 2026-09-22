import type { Settings, CustomTheme } from './types'

/** The built-in dark palette, also the starting point for a custom theme. Mirrors `app.css` `:root`. */
export const DEFAULT_CUSTOM: CustomTheme = {
  bg: '#0f1216',
  panel: '#171b21',
  panel2: '#1e242c',
  line: '#2a313b',
  text: '#e7ecf2',
  muted: '#97a3b2',
  accent: '#4c8dff',
}

export const DEFAULT_SETTINGS: Settings = {
  ai_enabled: true,
  keep_previews: true,
  theme: 'dark',
  custom: { ...DEFAULT_CUSTOM },
}

const CUSTOM_VARS: [keyof CustomTheme, string][] = [
  ['bg', '--bg'],
  ['panel', '--panel'],
  ['panel2', '--panel-2'],
  ['line', '--line'],
  ['text', '--text'],
  ['muted', '--muted'],
  ['accent', '--accent'],
]

/** Applies the chosen theme by toggling `data-theme` (built-ins) or setting inline CSS variables. */
export function applyTheme(settings: Settings) {
  const root = document.documentElement
  // Drop any custom variables from a previous choice so switching back to a built-in is clean.
  for (const [, cssVar] of CUSTOM_VARS) root.style.removeProperty(cssVar)
  if (settings.theme === 'custom') {
    // A custom theme builds on the dark base (for the hover variables it does not expose).
    root.dataset.theme = 'dark'
    for (const [key, cssVar] of CUSTOM_VARS) {
      const value = settings.custom?.[key]?.trim()
      if (value) root.style.setProperty(cssVar, value)
    }
  } else {
    root.dataset.theme = settings.theme
  }
}
