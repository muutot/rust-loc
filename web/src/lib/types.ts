// Mirror of the Rust DTOs in src/web/dto.rs and src/web/api.rs.

export interface Smells {
  unwrap: number
  expect: number
  panic: number
  todo: number
  unimplemented: number
  allow: number
  total: number
}

export interface Summary {
  lines: number
  blank: number
  comment: number
  doc: number
  code: number
  test: number
  physical: number
  complexity: number
  cognitive: number
  unsafe_kw: number
  test_fns: number
  /** Test functions across the whole tree: business scope plus the files
   * the scan excluded as test-only (the way Go projects keep them). */
  test_fns_all: number
  smells: Smells
  volume: number
  uloc: number | null
}

export interface FileRow {
  path: string
  module: string
  lang: string
  lines: number
  blank: number
  comment: number
  doc: number
  code: number
  test: number
  physical: number
  complexity: number
  cognitive: number
  smells: Smells
  uloc: number | null
  generated: number // 0/1/2
}

export interface ExcludedPath {
  path: string
  lines: number
}

export interface FunctionRow {
  path: string
  name: string
  line: number
  code: number
  complexity: number
  cognitive: number
  params: number
  depth: number
}

export interface Cocomo {
  project: string
  effort: number
  schedule: number
  people: number
  cost: number
}

export interface Hotspot {
  path: string
  code: number
  complexity: number
  commits: number
  churn: number
  score: number
}

export interface CouplingPair {
  a: string
  b: string
  commits: number
}

export interface Coupling {
  commits: number
  pairs: CouplingPair[]
}

export interface Gate {
  spec: string
  value: number
  limit: number
  op: string
  ok: boolean
  error: string | null
}

export interface Metrics {
  index: number
  volume: number
  cognitive: number
  sloc: number
  operators: number
  operands: number
  tokens: number
  methods: number
  arguments: number
  band: string
}

export interface LanguageRow {
  name: string
  files: number
  lines: number
  blank: number
  comment: number
  code: number
}

export interface Dupes {
  window: number
  duplicated_lines: number
  code_lines: number
  percent: number
  clusters: { lines: number; sites: { path: string; line: number }[] }[]
}

export interface BaselineRow {
  lines: number
  code: number
  complexity: number
  smells: number
}

export interface Diff {
  path: string
  old: BaselineRow
  new: BaselineRow
  files: { path: string; old: BaselineRow | null; new: BaselineRow | null; delta: number }[]
}

export interface Sample {
  when: number
  files: number
  lines: number
  blank: number
  comment: number
  code: number
  complexity: number
  smells: number
}

export interface Report {
  roots: string[]
  generated_at: number
  elapsed: number
  summary: Summary
  all_lines: number
  uloc_total: number | null
  files: FileRow[]
  excluded_files: number
  excluded_file_lines: number
  excluded_item_lines: number
  excluded_paths: ExcludedPath[]
  generated_files: number
  generated_lines: number
  functions: FunctionRow[]
  function_count: number
  cocomo: Cocomo | null
  hotspots: Hotspot[]
  coupling: Coupling | null
  gates: Gate[]
  metrics: Metrics | null
  languages: LanguageRow[]
  duplicates: Dupes | null
  diff: Diff | null
  history: Sample[]
  git_available: boolean
}

export interface AuthorRow {
  name: string
  email: string
  files: number
  blank: number
  comment: number
  doc: number
  code: number
  test: number
}

export interface AuthorShare {
  name: string
  email: string
  blank: number
  comment: number
  doc: number
  code: number
  test: number
}

export interface ActivityRow {
  name: string
  commits: number[]
}

export interface Activity {
  months: string[]
  rows: ActivityRow[]
}

export interface Authors {
  available: boolean
  untracked: number
  rows: AuthorRow[]
  files: { path: string; shares: AuthorShare[] }[]
  activity: Activity | null
}

export type ScanStatus =
  | { kind: 'idle' }
  | { kind: 'scanning'; stage: string; done: number; total: number; path: string }
  | { kind: 'done'; elapsed: number }
  | { kind: 'failed'; error: string }

export type AuthorsStatus =
  | { kind: 'idle' }
  | { kind: 'blaming'; done: number; total: number }
  | { kind: 'ready' }

export interface StatusSnapshot {
  scan: ScanStatus
  authors: AuthorsStatus
}

export interface Config {
  roots: string[]
  gitignore: boolean
  default_ignores: boolean
  include_tests: boolean
  complexity: boolean
  uloc: boolean
  exclude_generated: boolean
  cocomo: boolean
  authors: boolean
  exclude: string[]
  extra_dirs: string[]
  exclude_ext: string[]
  fail_if: string[]
  max_file_size: number
  sources: string[]
  lang: number
}

export interface DirListing {
  path: string
  parent: string | null
  entries: { name: string; path: string }[]
}

export interface LangInfo {
  slug: string
  name: string
  extension: string
}
