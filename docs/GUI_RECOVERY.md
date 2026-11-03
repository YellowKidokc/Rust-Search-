# Collection GUI and Windows launchers

The repository's branches contain different implementations:

| Ref inspected on 2026-11-02 | Contents |
| --- | --- |
| `claude/sleepy-hamilton-b5v9e4` (`df8ed160`) | Current collection engine, SQLite passage search, provenance and quotation verification |
| `codex/add-extended-file-type-parsers` (`126524e4`) | Earlier web GUI, named JSON indexes, extra parsers and event watcher |
| Default `OBS-Plugin-Final-Claude` (`60c08516`) | Older Markdown-only engine |

The default branch is not the most feature-complete branch. A normal pull of the
default branch does not recover the complete application.

The original GUI's HTML, Rust server, and README are preserved in `docs/recovered_gui`
from immutable commit `126524e491a578b6746da82b00e26d1799b54bf2`.
They are reference source, not active modules: its JSON index and result schema differ
from the SQLite collection engine. The current GUI adapts the earlier interface's
search/filter/saved-search/selection/export functions to the installed collection engine.

## Run

Double-click `SEARCH.bat` for the browser GUI. Python 3.11+ is required, with no extra
packages. It starts a localhost-only server on port 9800, opens your browser and calls
the Rust binary directly for search. `SEARCH_CLI.bat` offers interactive search without
a server. Neither launcher downloads, classifies, or reindexes material.

Build with `cargo build --release`, or set `TPSEARCH_EXE` to an existing compatible
binary. In David's copied desktop deployment, the existing D-drive binary is also
detected as a fallback. Set `TPSEARCH_CONFIG`, pass `--config PATH`, or use the default
per-user collection config. When this repo lives beside the central hub's
`tpsearch.toml`, that hub config is detected automatically.

The GUI supports collection, path, file-type, date, representation and existing
metadata filters; exact phrase/any-term search; source passage and context;
quotation status and duplicate provenance; browser-local saved searches; selected
passage Markdown/CSV/JSON downloads. It does not expose arbitrary filesystem writes.
It does not yet restore the earlier GUI's file-copy export or numeric grading filters.
Those grades must exist as source metadata before they can be filtered meaningfully.

If no collections exist, the UI explains which config it checked. Create and index a
collection before expecting search results. The central hub's baseline scan is not
a content index and performs no categorization. No automatic topic/argument scoring
was added by this work. GUI result order is lexical relevance, not truth or quality.

## Validation

`python ui/test_gui.py` exercises actual Rust search with temporary fixture collections,
including source/quote fields, existing metadata filtering, invalid filters and literal
query handling. For managed Windows environments, set `TPSEARCH_TEST_TMP` to an allowed
temporary directory before running tests. Browser rendering and local HTTP access must
be checked on the desktop; the managed Work session blocks localhost sockets.

The desktop working copy also preserves the five pre-existing uncommitted files from
the D-drive checkout. They were not silently discarded or treated as upstream commits.
The D-drive checkout itself remains unchanged because it is read-only to this session.
