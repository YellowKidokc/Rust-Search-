# Implementation status

## Starting point (inspected before stage 1)

The repository held a ~1,100-line Rust CLI (`index`, `query`, `repl`) for Theophysics Markdown:

* **Worked:** YAML frontmatter and truth-predicate/definition table parsing; a JSON index under
  `<root>/.tpsearch/`; mtime-based incremental reindex; substring search over body/frontmatter;
  hardcoded score filters (`s01`–`s10`, `evd_*`); saved searches; copy/CSV/manifest export. Two unit tests.
* **Gaps:** Markdown only; no collections or settings; a linear case-insensitive `contains` instead of
  a full-text index (no phrases, ranking or passages); one snippet per document with no line numbers;
  no hashing, provenance or duplicate handling; records under unreachable roots were deleted on reindex;
  the index was written inside the source tree; the schema was Theophysics-specific; no integration tests.
  The README's description matched the code; its "later phases" were not started.

The legacy commands are preserved unchanged in behaviour (covered by `legacy_markdown_index_still_works`).

## Stage 1: delivered

| Requirement | State |
|---|---|
| Named collections with multiple folders, drives, UNC paths | done |
| Excludes (globs), file types, max size, hidden, symlinks/junctions | done |
| Manual / scheduled / watched | manual; `watch` = incremental re-index on an interval; Task Scheduler recipe documented. **No filesystem-event watcher.** |
| Concurrency limit, throttle, pause/resume | done (`threads`, `throttle_ms`, `pause`/`resume`) |
| Index storage location | done (`index_dir`; default per-user data dir) |
| Analysis depth per collection/root | names / text / metadata selectable; semantic and AI flags exist and are **rejected** |
| Offline roots kept, not deleted | done, tested |
| Loop and overlapping-root protection | done, tested |
| Adapters: MD, TXT (+code/config), JSON, YAML, SRT, VTT, HTML, DOCX | done (HTML/DOCX are dependency-free extractors: fine for ordinary documents, not for every edge case) |
| PDF | done (text extraction; image/scanned PDFs detected and reported, OCR not yet implemented) |
| Extracted text outside originals; failures/unsupported visible | done (`status`) |
| JSON/YAML analyses as derived records linked to sources | done when the file declares its source |
| Keyword, phrase, prefix, exclusion search; filters (collection, path, folder, type, date, metadata, representation) | done |
| Passage hits with context, why-matched, source locations, open | done |
| Relevance kept separate from truth/quality/confidence | done (labelled; no truth fields exist) |
| Provenance: path, SHA-256, extractor+version, locations | done |
| Copies collapse; independent-source counts | done, tested |
| Quote validation against source | done for every displayed hit |
| Dry-run inventory | done |
| Incremental, recover from interruption, progress, errors, stale, offline | done; interruption tested manually (SIGKILL mid-run) |
| Folder-report JSON export | done (`tpsearch.folder_report/v1`) |

## Not yet implemented (honest list)

1. **OCR** for image/scanned PDFs.
2. **Semantic search** (local embedding model, passage vectors) and **hybrid** lexical+semantic ranking.
3. **Local AI classification/extraction** and **paid API enrichment** with provider/model/scope/spend
   enforcement and an explicit external-send gate (settings exist; all are rejected today).
4. **Metadata profiles** (research, arguments, transcripts, code, general), multiple classifications
   per passage, manual corrections, machine-vs-human review state. The export reserves the field.
5. **Question workspace** (gather across collections, group by topic/argument, supports/challenges/gaps,
   cited synthesis).
6. **Versions and summaries as representations**: only original / copy / transcript / derived exist.
   Near-duplicate (non-identical) detection is not implemented.
7. **Real-time filesystem watching** (`notify`/ReadDirectoryChangesW); `watch` polls.
8. **A native desktop GUI.** The collections web interface runs locally in a browser.
9. DOCX page mapping; DOCX locations are paragraph numbers. PDF passages include page numbers.
10. Cross-collection ranking: BM25 scores from different collections are not strictly comparable.
11. Verified on Linux; the Windows GNU release executable cross-builds on Linux. Native Windows
    execution remains unverified (hidden attributes, junction handling, UNC paths, desktop viewers).

## Stage 2: Windows build and collections web UI

- Repaired the merged CLI command wiring and restored the dependencies already used by the
  collection implementation. Legacy `query`, `watch --name`, and `serve --index-name` remain available.
- `tpsearch serve -c research --port 9800` serves the newer David-OS interface from
  `ui/search.html`. Add `--open` to launch the browser. It binds only to `127.0.0.1`.
- Collection search supports multiple collections, phrase/prefix/exclusion queries and the CLI
  filters. The interface includes collection health, OCR-needed PDF counts, background indexing
  and dry-run previews. PDF results show their page number alongside their section heading.
- Windows hidden attributes are detected; reparse points are skipped when links are disabled.
  Canonical directory identities prevent junction loops when links are followed. Offline UNC
  roots are preserved. Editor launchers such as `code.cmd` are resolved from PATH on Windows.
- `SEARCH.bat` starts the web UI; `SEARCH_CLI.bat` forwards arguments to the release binary.
- Windows-specific hidden-attribute, junction-loop and UNC tests require a native Windows runner.
  Cross-compilation does not verify actual NTFS, network-share, or desktop-viewer behavior.
