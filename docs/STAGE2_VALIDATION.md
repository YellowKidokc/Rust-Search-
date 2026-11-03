# Stage 2 validation

The newer `ui/search.html` David-OS design remains the interface. No Node build system,
framework, database migration, or search/adapter algorithm change is required.

## Platform audit

- Unix-only filesystem calls occur in Unix-gated tests. Production code has no direct libc
  or POSIX-only API dependency.
- Windows hidden-file detection already used `MetadataExt::file_attributes() & 0x2`;
  a Windows-gated attribute test now covers it.
- Junction detection additionally checks reparse-point attributes. Disabled link following
  prunes those entries before descent; enabled following tracks canonical directory identities
  to stop junction loops. A Windows-gated test creates an actual directory junction.
- Path operations use `Path`/`PathBuf`. Slash-normalized relative paths are intentional for
  glob matching and file URIs. Unreachable UNC roots are retained verbatim; a Windows-gated
  test checks the UNC prefix. No live SMB share is available in this Linux environment.
- `open_with` substitutes source paths after tokenizing the configured template, retaining
  spaces in the argument. Windows PATH resolution includes `.cmd` editor launchers.
  The HTTP integration test verifies a configured viewer receives the actual PDF path on Unix.
- Both Windows batch launchers invoke `target\release\tpsearch.exe` and preserve its exit code.

## Checks

```bash
cargo test --locked
cargo build --release --locked
cargo build --release --locked --target x86_64-pc-windows-gnu
cargo test --locked --target x86_64-pc-windows-gnu --no-run
```

The Windows cross-build uses Rust 1.99.1 and MinGW GCC 14.2. Native Windows test execution
is still required; cross-compiling tests does not execute them.

The real HTTP test covers two indexed collections, default/all selection, phrase and prefix
search, PDF page numbers, metadata filtering, status/type/OCR counts, malformed requests,
viewer launch, dry-run and actual indexing jobs, and rejection of foreign hosts/origins.
Chromium checks at 1440×1000 and 390×844 cover search, PDF labels, collection health,
index preview, JavaScript errors, and horizontal overflow.
