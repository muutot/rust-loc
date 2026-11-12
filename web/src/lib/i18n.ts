// UI text, straight from the catalog the `--dump-i18n` flag dumped: zh.json and
// en.json are generated from src/i18n.rs, so the three surfaces cannot drift.
// The saved `lang` index picks the file, same index the GUI's INI stores.

import zh from '../i18n/zh.json'
import en from '../i18n/en.json'

type Catalog = Record<string, string>

const CATALOGS: Catalog[] = [zh as Catalog, en as Catalog]
export const LANG_NAMES = ['中文', 'English']

export function t(lang: number, key: string): string {
  return CATALOGS[lang === 1 ? 1 : 0][key] ?? key
}

export function catalog(lang: number): Catalog {
  return CATALOGS[lang === 1 ? 1 : 0]
}

/// Replace `{name}` placeholders, the way the Rust `i18n::fill` does. Unknown
/// braces pass through untouched, so a stray `{` never eats a string.
export function fill(template: string, parts: Record<string, string | number>): string {
  return template.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in parts ? String(parts[name]) : whole,
  )
}
