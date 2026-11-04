# tpsearch — local-first desktop and drive search

`tpsearch` is a fast, local search engine for research documents. It indexes Markdown, text, JSON, YAML, PDF, DOCX, and HTML into an incrementally updated JSON index and provides both a CLI and a local web interface.

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

## Tests

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

## Collections web interface

```bash
tpsearch serve -c research --port 9800 --open
```

This serves the David-OS interface in `ui/search.html`. Choose a collection or search all
collections; the Status tab shows extraction errors, offline roots, and PDFs needing OCR.
Use **Preview indexing** for a dry run or **Index collection** for an incremental background job.
PDF hits show page numbers. Clicking a passage's file name opens its source through `open_with`
or the operating system's default viewer. The server accepts only loopback, same-origin requests.

`SEARCH.bat` launches this interface on Windows. `SEARCH_CLI.bat` forwards CLI arguments.
Both use `target\release\tpsearch.exe`; build with `cargo build --release` first.

Collections HTTP endpoints (JSON):

- `GET /api/collections`: names, roots, indexing state, and status summaries.
- `GET /api/search?q=mercy&collection=research,notes&type=pdf&phrase=false`:
  CLI-shaped search results, one result object per indexed collection. Other query options are
  `any`, `path`, `under`, `after`, `before`, `meta=key=value`, `representation`, `limit`,
  `include_missing`, and `no_verify`. Omit `collection` to use the launch default;
  pass an empty collection to search all. Commas separate collection names and file types.
- `POST /api/search`: the existing UI request (`collections`, `query`, `types`, `meta` arrays and
  the corresponding filter fields); an empty collections array searches all collections.
- `GET /api/status?collection=research`: CLI status plus `files_by_type` and `pdf_needs_ocr`.
- `POST /api/index?collection=research&dry_run=true`: start a job. Poll
  `GET /api/index?collection=research` for `running`, `completed` with CLI-shaped `result`,
  or `failed` with an `error`. Job state is held in memory and resets when the server restarts.
- `POST /api/open` with `{"collection":"research","passage_id":123}`: open an indexed source.
- `POST /api/folder-report` with `collection` and `folder`: folder provenance report.

The Windows cross-build target is `x86_64-pc-windows-gnu` and requires the matching Rust target
and MinGW GCC. Native Windows tests are still required to validate NTFS and UNC behavior.

## Local semantic search (Stage 3)

Semantic search uses a **local embedding service**. It does not call a paid API and does not
change FTS5 keyword search. ONNX Runtime's default package does not provide binaries for the
Windows GNU target used here, so this build uses the local-API fallback rather than bundling
inference. The service manages model files, downloads, and caching.

Install [Ollama](https://ollama.com), then prepare a CPU-only embedding service:

```bash
# In one terminal (set OLLAMA_NUM_GPU=0 in your Windows environment settings instead).
OLLAMA_NUM_GPU=0 ollama serve
# In another terminal:
ollama pull nomic-embed-text

tpsearch index -c research
tpsearch embed -c research --dry-run
tpsearch embed -c research --api-url http://localhost:11434 --model nomic-embed-text --batch-size 32
tpsearch semantic -c research "quantum error correction" --top 10 --threshold 0.3
```

`nomic-embed-text` is the fallback default. Vector dimensions are validated against the service
response and saved with the model; they are not hardcoded to MiniLM's 384 dimensions.
Ollama requests disable GPU use and text truncation. For any other local service, configure its
CPU provider yourself. No GPU is required by tpsearch.

A local OpenAI-compatible service is also supported (this is a protocol, not a paid provider):

```bash
# Start your CPU service with all-MiniLM-L6-v2 installed under this model alias first.
tpsearch embed -c research --api-url http://127.0.0.1:8080/v1/embeddings --model all-MiniLM-L6-v2
tpsearch semantic -c research "quantum error correction" --json
```

Only plain HTTP on loopback (`localhost`, `127.0.0.1`, or `::1`) is accepted. Redirects and
remote/paid endpoints are rejected. Source passages are sent only to that selected local
service. `--model-path` and bundled model downloading are not used in this fallback; configure
local model files in your inference service. Different model names keep independent vectors.
Treat model aliases as immutable: if weights change, choose a new alias and re-embed. The same
alias cannot silently switch endpoints or vector dimensions.

The model, endpoint and dimension count are saved in the collection's `embedding` settings
when embedding completes successfully. Subsequent `embed`/`semantic` commands reuse them.
Already embedded passages are skipped; with no changes, `embed` makes **zero inference calls**
and works with the service stopped. Re-indexing invalidates embeddings for changed passages;
missing files are excluded from semantic results. Existing index databases gain the new tables
and invalidation triggers automatically; existing tables and FTS5 logic are unchanged.

Use `--json` for embedding counts/errors or ranked semantic results. Failed batches return a
nonzero exit status; successful batches remain stored so a retry can resume. A failed model
change does not replace the active collection model. `--dry-run` reports pending passages
without inference or changing collection settings. Ranking uses brute-force cosine similarity;
thresholds are between -1 and 1. For larger collections, `tpsearch semantic -c research "query" --index` builds/reuses an
optional IVF index (spherical k-means, stored in separate SQLite tables). Changed embeddings
invalidate it automatically. Candidate selection is approximate; omit `--index` for exhaustive
cosine ranking. Hybrid RRF search remains a stretch goal and is not implemented.

Tests use a deterministic local HTTP fixture (including 384-dimensional responses) to check
API handling, ordering, persistence and invalidation. They do not measure real model quality.
