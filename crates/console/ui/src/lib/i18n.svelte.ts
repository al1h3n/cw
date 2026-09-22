import { invoke } from './bridge'

export interface LanguageOption {
  code: string
  name: string
}

interface Translation {
  code: string
  strings: Record<string, string>
  available: LanguageOption[]
  problems: string[]
  languages_dir: string
}

/**
 * Translations come from the Rust side, which reads `.ini` files, so adding a language never needs
 * a rebuild. Everything here is reactive: switching language re-renders the whole window.
 */
class I18n {
  code = $state('en')
  strings = $state<Record<string, string>>({})
  available = $state<LanguageOption[]>([])
  problems = $state<string[]>([])
  languagesDir = $state('')
  ready = $state(false)

  /** Loads a language; `undefined` means "remembered choice, else OS language, else English". */
  async load(code?: string) {
    const result = await invoke<Translation>('translation', {
      code: code ?? null,
      system: navigator.language,
    })
    this.code = result.code
    this.strings = result.strings
    this.available = result.available
    this.problems = result.problems
    this.languagesDir = result.languages_dir
    this.ready = true
  }

  /** Switches language and remembers the choice for next launch. */
  async choose(code: string) {
    await invoke('set_language', { code })
    await this.load(code)
  }

  /** Writes the English file into the languages folder for a translator to edit. */
  async exportTemplate(): Promise<string> {
    return invoke<string>('export_language_template')
  }

  /** Looks up a key and substitutes {0}, {1}, … */
  t(key: string, ...args: (string | number)[]): string {
    const template = this.strings[key]
    if (template === undefined) return key
    return template.replace(/\{(\d+)\}/g, (whole, index) => {
      const value = args[Number(index)]
      return value === undefined ? whole : String(value)
    })
  }
}

export const i18n = new I18n()

/** Shorthand so markup stays readable: {t('room')} */
export const t = (key: string, ...args: (string | number)[]) => i18n.t(key, ...args)
