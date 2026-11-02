# Collection settings

Settings live in one TOML file **outside every indexed root**:

| Platform | Default path |
|---|---|
| Windows | `%APPDATA%\tpsearch\config.toml` |
| macOS | `~/Library/Application Support/tpsearch/config.toml` |
| Linux | `~/.config/tpsearch/config.toml` |

Override with `--config FILE` or `TPSEARCH_CONFIG`. `tpsearch collection config-path` prints the active path.
Each collection's index (`index.sqlite`) defaults to `%LOCALAPPDATA%\tpsearch\collections\<name>\`
(override the base with `TPSEARCH_DATA`, or per collection with `index_dir`).

Edit the file directly, or use `tpsearch collection set NAME KEY VALUE` for top-level keys.
Settings this build cannot honour are **rejected**, not silently ignored.

## Example

```toml
[collections.research]
max_file_size_mb = 50
hash_max_size_mb = 4096
include_hidden = false
follow_symlinks = false
mode = "manual"
interval_minutes = 60
threads = 2
throttle_ms = 0
extensions = []            # empty = inventory every file
exclude = ["**/.git/**", "**/node_modules/**", "**/archive/**", "*.tmp"]
profiles = []
open_with = "code -g {path}:{line}"

[[collections.research.roots]]
path = 'D:\Research'

[[collections.research.roots]]
path = '\\192.168.1.177\share\papers'

[[collections.research.roots]]
path = 'E:\'
[collections.research.roots.depth]   # per-root override: names/metadata only
names = true
text = false
metadata = false

[collections.research.depth]
names = true
text = true
metadata = true
semantic = false
local_ai = false
paid_api = false

[collections.research.ai]
provider = "none"
allow_external = false
max_spend_usd = 0.0
scope = "none"
```

## Keys

| Key | Default | Meaning |
|---|---|---|
| `roots` | — | Folders, drive roots (`E:\`) or UNC/NAS paths. Each may carry its own `depth`. |
| `exclude` | `.git`, `node_modules`, `.tpsearch`, `$RECYCLE.BIN`, `System Volume Information`, `target`, `~$*` | Globs, case-insensitive, matched against the root-relative path with `/` separators. Excluded folders are not descended. |
| `extensions` | `[]` | Allow-list of extensions (no dot). Empty = every file is inventoried; only supported types are extracted. |
| `max_file_size_mb` | 50 | Bigger files are inventoried (name, size, hash) with status `too_large`; not extracted. |
| `hash_max_size_mb` | 4096 | Bigger files get no SHA-256 (so no duplicate detection) to spare NAS bandwidth. |
| `include_hidden` | false | Dot-files/folders and (Windows) files with the Hidden attribute. |
| `follow_symlinks` | false | Follow symbolic links **and Windows directory junctions** (Rust reports both as links). When following, loops are detected and reported; each real file is ingested once. |
| `mode` | manual | `manual`: run `tpsearch index -c NAME`. `scheduled` / `watched`: run `tpsearch watch -c NAME` (re-index every `interval_minutes`), or register a Windows Task Scheduler job (below). |
| `interval_minutes` | 60 | Interval for `watch`. |
| `threads` | 2 | Worker threads for hashing and extraction. This is the CPU/IO concurrency limit. |
| `throttle_ms` | 0 | Sleep per worker between files, to reduce pressure on NAS volumes. |
| `index_dir` | per-user data dir | Where `index.sqlite` lives. If placed inside a root it is excluded automatically. |
| `open_with` | system default | Command template for `tpsearch open`; `{path}` and `{line}` are substituted. |
| `depth.names` | true | Always on: name, path, size, dates, content hash. |
| `depth.text` | true | Extract document text into passages. |
| `depth.metadata` | true | Index existing structured metadata (frontmatter, JSON/YAML fields, DOCX properties). |
| `depth.semantic` / `depth.local_ai` / `depth.paid_api` | false | **Rejected in this build.** Reserved for later stages. |
| `ai.provider` | "none" | Must be `none` in this build. `allow_external` must be true before any later stage may send content off the machine; `max_spend_usd` and `scope` bound paid use. |
| `profiles` | `[]` | Reserved for optional metadata profiles (research, arguments, transcripts, code, general). |

## Pause / resume

`tpsearch pause -c NAME` creates a `PAUSE` file in the index directory; workers finish their current file and wait.
`tpsearch resume -c NAME` removes it. Killing an index run is safe: work is committed in batches, the next run
reports the interruption and only processes what is left.

## Scheduling on Windows

```bat
schtasks /Create /SC HOURLY /TN "tpsearch research" /TR "C:\Tools\tpsearch.exe index -c research"
```

Real-time filesystem-event watching is not implemented yet; `watch` polls at `interval_minutes`, and each pass is incremental.
