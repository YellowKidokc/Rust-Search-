# Folder report export — `tpsearch.folder_report/v1`

`tpsearch export-folder` emits a JSON description of one folder from the index, intended as the data
source for a separately generated per-folder HTML report (for example an FIS-style `FIS.html`).
tpsearch **does not generate HTML and does not write into indexed folders**; the search engine works
without any report consumer.

This contract was designed from a presentation reference (an exported `FIS.html` page). It is not
derived from, or tested against, the original FIS generator or its registry, which are not part of
this repository. Fields FIS shows that tpsearch does not produce (compressed names, lossless addresses,
CHI vectors, law signals, canon standing, rename proposals) are deliberately absent; a consumer
that needs them must compute them itself and keep them labelled as its own derived layer.

```bash
tpsearch export-folder -c research "D:\Research\FABLE" --out fable.json
tpsearch export-folder -c research "D:\Research" --recursive --out reports\
```

`--recursive` writes one `FLD-xxxxxxxxxxxx.json` per folder that contains indexed files (plus their
ancestors) and a `manifest.json` (`tpsearch.folder_report_manifest/v1`) mapping folder paths to report files.

## Shape

```jsonc
{
  "schema": "tpsearch.folder_report/v1",
  "generator": {"name": "tpsearch", "version": "0.1.0"},
  "generated_at": "2026-10-29T00:00:00Z",
  "collection": "research",
  "folder": {
    "folder_id": "FLD-49eeb1bda738",      // sha256(path)[..12]; stable while the path is stable
    "path": "D:\\Research\\FABLE", "name": "FABLE", "uri": "file:///D:/Research/FABLE",
    "root": "D:\\Research", "root_status": "online",   // or "offline": records retained, not re-verified
    "direct_files": 4, "child_folders": 0, "bytes": 28331,
    "missing_files": 0,                    // stale records: not found on last scan of an online root
    "independent_sources": 4,              // copies and linked derived records count once
    "counts_by_ext": {"html": 2, "md": 2},
    "counts_by_status": {"indexed": 4}
  },
  "scan": {"run": 12, "started": "…", "finished": "…", "status": "complete"},  // latest run of the collection
  "files": [
    {
      "file_id": 41, "stable_id": "FIL-ce5f481725a9",
      "name": "Cathedral-Shell-v2.html", "path": "D:\\Research\\FABLE\\Cathedral-Shell-v2.html",
      "rel_path": "FABLE/Cathedral-Shell-v2.html", "uri": "file:///D:/Research/FABLE/Cathedral-Shell-v2.html",
      "ext": "html", "size": 5630, "modified": "2026-07-22T03:21:20Z", "first_indexed": "…",
      "sha256": "563e81…",                // null if over hash_max_size_mb
      "status": "indexed",                 // indexed | metadata_only | pending_adapter | too_large | failed
      "status_reason": null,
      "extractor": {"name": "html", "version": 1},
      "passage_count": 12,
      "representation": "original",        // original | copy | transcript | derived
      "copy_of": null,                     // {"file_id", "path"} of the first-indexed identical file
      "derived_from": null,                // {"declared", "file_id", "path"} for JSON/YAML analysis records
      "metadata": {"origin": "source", "fields": {"title": ["…"]}},   // read from the file itself
      "classifications": []                // reserved; see below
    }
  ],
  "child_folders": [
    {"folder_id": "FLD-…", "path": "…", "name": "…", "uri": "…", "files_recursive": 3, "files_direct": 1}
  ],
  "duplicate_groups": [ {"sha256": "…", "paths": ["…", "…"]} ]   // byte-identical, anywhere in the collection
}
```

## Semantics a consumer must preserve

* **Source identity is the SHA-256 of the bytes.** `representation = "copy"` means byte-identical to an
  earlier-indexed file; which member is called "original" is a bookkeeping choice (earliest indexed),
  not a claim about creation order.
* **`derived_from`** is only set when a JSON/YAML file declares its source (`source_path`, `source_file`,
  `original_path`, `source`, `source_sha256`, `sha256`). The link resolves to the first indexed file
  matching by hash, absolute path, root-relative path, or bare file name (a bare name can be ambiguous);
  `file_id: null` means declared but unresolved.
* **Metadata `origin: "source"`** values come from the file; nothing in v1 is machine-generated.
* **`classifications`** is reserved. Later stages will populate entries shaped
  `{"profile", "field", "value", "passage_id"?, "origin": "machine"|"human", "model"?, "confidence"?, "reviewed": bool}`,
  keeping machine proposals distinct from human-reviewed values and preserving unknown values.
* Offline roots: reports still list their files with `root_status: "offline"`.

Additive fields may appear within v1; removals or meaning changes bump the schema version.
