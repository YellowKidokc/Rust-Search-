# tpsearch — local-first desktop and drive search

`tpsearch` indexes named **collections** of local folders, whole drives and UNC/NAS paths into a
per-collection SQLite full-text index, and returns **passage-level** results with source locations,
context, provenance and duplicate handling. Everything runs locally; no AI provider is used or needed.

Source files are only ever read. tpsearch never renames, moves, deletes or rewrites them, and
collection indexes live outside the indexed roots.

The original Theophysics Markdown commands (`index PATH`, `query`, `repl`) are unchanged; see
[Legacy Markdown mode](#legacy-markdown-mode).

Status: **stage 1** (collections + reliable full-text search). See [docs/STATUS.md](docs/STATUS.md)
for what is implemented and what is not.

## Build

```bash
cargo build --release
```

The binary is `target/release/tpsearch` (`tpsearch.exe` on Windows; SQLite is compiled in, so no
separate install). Building on Windows needs the MSVC build tools (for SQLite's C source).

## Quick start

```bat
tpsearch collection new research --root "D:\Research" --root "\\192.168.1.177\share\papers"
tpsearch collection exclude research "**/archive/**"
tpsearch index -c research --dry-run      :: inventory first; writes nothing
tpsearch index -c research                :: incremental; safe to interrupt and re-run
tpsearch search -c research grace field
tpsearch status -c research
```

## Searching

```bat
tpsearch search -c research "free will" entropy            :: phrase AND keyword
tpsearch search -c research --phrase "moral conservation"  :: whole query as one phrase
tpsearch search -c research coherence decoherence --any    :: any term
tpsearch search -c research logos -chaos                   :: exclude a term
tpsearch search -c research entrop*                        :: prefix
tpsearch search -c research grace --type md,docx --under "D:\Research\Laws" --after 2026-01-01
tpsearch search -c research grace --meta "author=Lowe" --representation original
tpsearch search -c research -c transcripts grace           :: several collections (or --all)
tpsearch search -c research grace --json                   :: machine-readable
```

Each hit shows:

* the file and location: **lines** (text, Markdown, code, JSON/YAML, HTML), **paragraph** (DOCX),
  or **timestamp** plus lines (SRT/VTT), and the enclosing **section heading**;
* the passage with the neighbouring passages as context;
* **why it matched**: which terms, and whether in text, a section heading, or metadata;
* a **relevance** number. This is a BM25 lexical rank only, not truth, evidence quality or
  classification confidence;
* a **quote check**: the displayed text is re-read from the source file and confirmed (line formats),
  or the source hash is confirmed unchanged (HTML/DOCX), or the result is flagged as changed/unavailable;
* identical copies collapsed into one hit, and the header counts **independent sources**:
  byte-identical copies and JSON/YAML analyses that declare their source count once;
* an `open` command: `tpsearch open -c research <passage-id>` opens the file (set `open_with`,
  e.g. `code -g {path}:{line}`, to jump to the line); `--print` prints `path:line`.

Files whose **name or path** matches are listed separately below the passages.

## Supported formats (stage 1)

| Format | Extraction | Location |
|---|---|---|
| Markdown | text + YAML frontmatter as metadata | line, heading |
| TXT, CSV/TSV, logs, source code, TOML/INI/XML | text | line |
| JSON, YAML | text + flattened fields as metadata; `source_path`-style keys mark the file as **derived** from that source | line |
| SRT, VTT | cue text | line + timestamp |
| HTML | visible text (scripts/styles dropped), title | line, heading |
| DOCX | paragraphs, core properties | paragraph, heading style |
| PDF | **not yet**: inventoried as `pending_adapter` (visible in `status`) | — |
| anything else | name, path, size, dates, SHA-256 (`metadata_only`) | — |

Extraction failures, oversized files, unsupported and pending formats are recorded per file and
listed by `tpsearch status`.

## Incremental indexing and recovery

* Unchanged size + modification time + extractor version → nothing is read.
* Changed timestamp but identical SHA-256 → no re-extraction.
* Extractor upgrades re-extract only the affected formats.
* A file that fails is not retried until it changes.
* Work is committed in batches. If a run is killed, the next run reports it and continues.
* `tpsearch pause|resume -c NAME` pauses workers between files.
* **Offline roots** (unplugged drive, NAS down) are reported as offline. Their records stay
  searchable and are marked `ROOT OFFLINE`; nothing is deleted.
* Files missing from an *online* root become `missing` (stale) records, excluded from search unless
  `--include-missing`.
* Overlapping roots are ingested once (an inner root keeps its own settings). Symlink/junction loops
  are detected.

## Settings

See [docs/SETTINGS.md](docs/SETTINGS.md): roots, excludes, file types, size limits, hidden files,
symlinks/junctions, manual/scheduled/watched modes, threads/throttle, pause/resume, index location,
per-root analysis depth, and the (disabled) AI settings.

## Folder reports

`tpsearch export-folder -c NAME FOLDER [--recursive --out DIR]` emits documented JSON
(`tpsearch.folder_report/v1`, [docs/FOLDER_REPORT_EXPORT.md](docs/FOLDER_REPORT_EXPORT.md)) with the
folder's inventory, statuses, content hashes, copy/derived relationships, child folders and scan status,
for a separate per-folder HTML report generator. tpsearch itself writes no HTML.

## Legacy Markdown mode

```bash
tpsearch index /path/to/documents /another/file.md
cd /path/to/documents
tpsearch query --tag resurrection --min-score 70 --min-s07 5
tpsearch query --search "minimal facts" --sort evd_balance --limit 10
tpsearch query --domain Theology --has-counter true --json
tpsearch repl
```

This mode keeps its original behaviour: a JSON index in `.tpsearch/index.json` beneath the first root
(note that this *does* write into the root), Theophysics frontmatter fields (`score_total`, `s01`–`s10`,
`evd_*`, truth-predicate and definition tables), saved searches (`--save`, `--load`, `--frozen`,
`--list-saved`) and exports (`--select`, `--export-to`, `--export-manifest`, `--export-csv`, `--flatten`).
All query filters are AND-combined; sort keys are `score`, `s01`–`s10`, `evd_balance`, `title`, `modified`.

## Tests

```bash
cargo test
```

`tests/acceptance.rs` drives the binary end to end. It checks keyword search without AI, no
re-extraction of unchanged files, offline-root retention, copy collapsing and independent-source
counts, correct source line locations, byte-for-byte unchanged sources, overlapping roots, symlink
loops, and the legacy commands.
