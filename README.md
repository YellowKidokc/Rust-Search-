# Theophysics Search Engine

`tpsearch` is a fast, local search engine for research documents. It indexes Markdown, text, JSON, YAML, PDF, DOCX, and HTML into an incrementally updated JSON index and provides both a CLI and a local web interface.

## Build

Install a current stable Rust toolchain, then run:

```bash
cargo build --release
```

The single binary is `target/release/tpsearch` (`tpsearch.exe` on Windows).

## Index and query

```bash
tpsearch index /path/to/documents /another/file.md
tpsearch index --name habermas /path/to/transcripts
cd /path/to/documents
tpsearch query --tag resurrection --min-score 70 --min-s07 5
tpsearch query --search "minimal facts" --sort evd_balance --limit 10
tpsearch query --domain Theology --has-counter true --json
tpsearch repl
tpsearch query --index-name habermas --search "minimal facts"
tpsearch query --index-name all --tag resurrection
tpsearch indexes
```

An unnamed index lives in `.tpsearch/index.json` beneath the first root. Named indexes live in `~/.tpsearch/indexes/<name>/index.json`, or beneath `TPSEARCH_HOME` when that environment variable is set. Indexing again reuses unchanged records, updates changed files, and removes missing ones. Run queries inside an unnamed index root (or an ancestor below it), provide `--index /path/to/.tpsearch/index.json`, or select a named index with `--index-name`.

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
tpsearch query --search grace --export-snippets grace.md
```

`--select` accepts comma-separated numbers and ranges such as `1,3,5-8`. Export commands use all displayed results when neither interactive selection nor an explicit result set is otherwise needed. `--flatten` discards source directory structure when copying; by default relative paths beneath an indexed root are preserved.

## Watch mode and web interface

```bash
tpsearch watch --name habermas
tpsearch serve --port 9800 --index-name habermas
```

Watch mode batches filesystem changes for two seconds and refreshes the index. The web interface opens in the default browser and supports search, filters, saved searches, result selection, and file, CSV, snippet, and JSON exports. It binds to localhost only.
