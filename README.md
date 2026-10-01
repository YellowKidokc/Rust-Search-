# Theophysics Search Engine

`tpsearch` is a fast, local Phase 1 search engine for Markdown research documents. It indexes flexible YAML frontmatter, Markdown truth-predicate and definition tables, and document text into an incrementally updated JSON index. No server or external database is required.

## Build

Install a current stable Rust toolchain, then run:

```bash
cargo build --release
```

The single binary is `target/release/tpsearch` (`tpsearch.exe` on Windows).

## Index and query

```bash
tpsearch index /path/to/documents /another/file.md
cd /path/to/documents
tpsearch query --tag resurrection --min-score 70 --min-s07 5
tpsearch query --search "minimal facts" --sort evd_balance --limit 10
tpsearch query --domain Theology --has-counter true --json
tpsearch repl
```

The index lives in `.tpsearch/index.json` beneath the first root. Indexing again reuses unchanged records, updates changed files, and removes missing ones. Run queries inside that root (or an ancestor below it), or provide `--index /path/to/.tpsearch/index.json`.

All query filters are AND-combined. Supported filters include `--tag`, `--topic`, `--domain`, `--series`, `--claim`, `--search`, `--min-score`, `--min-s01` through `--min-s10`, `--min-evd-support`, and `--has-counter [true|false]`. Sort keys are `score`, `s01`–`s10`, `evd_balance`, `title`, and `modified`.

## Saved searches and export

```bash
tpsearch query --tag resurrection --save strong_resurrection
tpsearch query --list-saved
tpsearch query --load strong_resurrection
tpsearch query --load strong_resurrection --frozen
tpsearch query --tag axiom --select --export-to exports
tpsearch query --tag axiom --select-all --export-manifest manifest.json
tpsearch query --tag resurrection --export-csv results.csv
```

`--select` accepts comma-separated numbers and ranges such as `1,3,5-8`. Export commands use all displayed results when neither interactive selection nor an explicit result set is otherwise needed. `--flatten` discards source directory structure when copying; by default relative paths beneath an indexed root are preserved.

## Scope

This release implements Phase 1 (`.md`) only. PDF, DOCX, HTML, JSON/YAML document ingestion and the web UI are intentionally reserved for later phases.
