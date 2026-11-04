//! Collection-mode commands. Legacy Markdown commands stay in main.rs.
use crate::{config, db, report, scan, search};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Subcommand)]
pub enum CollectionCmd {
    /// Create a collection.
    New {
        name: String,
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
        #[arg(long)]
        index_dir: Option<PathBuf>,
    },
    List,
    Show {
        name: String,
    },
    /// Add a folder, drive (e.g. D:\) or UNC/NAS path (\\server\share\dir).
    AddRoot {
        name: String,
        path: PathBuf,
        /// Names, paths and filesystem metadata only for this root.
        #[arg(long)]
        names_only: bool,
    },
    RemoveRoot {
        name: String,
        path: PathBuf,
    },
    /// Add an exclude glob, e.g. "**/archive/**" or "*.tmp".
    Exclude {
        name: String,
        glob: String,
    },
    /// Change a setting, e.g. `set docs max_file_size_mb 100`.
    Set {
        name: String,
        key: String,
        value: String,
    },
    /// Remove a collection from the settings (its index directory is left in place).
    Remove {
        name: String,
    },
    /// Print the settings file path.
    ConfigPath,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Collections to search (repeatable). Omit with --all to search every collection.
    #[arg(short, long)]
    collection: Vec<String>,
    #[arg(long)]
    all: bool,
    /// Keywords, "quoted phrases", -excluded, prefix*.
    #[arg(required = true)]
    query: Vec<String>,
    /// Treat the whole query as one exact phrase.
    #[arg(long)]
    phrase: bool,
    /// Match any term instead of all terms.
    #[arg(long)]
    any: bool,
    /// File types, comma-separated (md,txt,pdf...).
    #[arg(long = "type", value_delimiter = ',')]
    types: Vec<String>,
    /// Substring of the full path.
    #[arg(long)]
    path: Option<String>,
    /// Only files below this folder.
    #[arg(long)]
    under: Option<String>,
    /// Modified on/after YYYY-MM-DD.
    #[arg(long)]
    after: Option<String>,
    /// Modified before YYYY-MM-DD.
    #[arg(long)]
    before: Option<String>,
    /// Metadata filter key=value (substring match on value). Repeatable.
    #[arg(long)]
    meta: Vec<String>,
    /// original | copy | transcript | derived
    #[arg(long)]
    representation: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Include records whose file was not found on the last scan.
    #[arg(long)]
    include_missing: bool,
    /// Skip re-reading sources to verify displayed quotations.
    #[arg(long)]
    no_verify: bool,
    #[arg(long)]
    json: bool,
}

fn load(cfg_path: &Path) -> Result<config::Config> {
    config::load(cfg_path)
}

fn root_path(p: &Path) -> PathBuf {
    // Keep UNC and unreachable paths as typed; canonicalize reachable local ones.
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn collection(cfg_path: &Path, cmd: CollectionCmd) -> Result<()> {
    let mut cfg = load(cfg_path)?;
    match cmd {
        CollectionCmd::New {
            name,
            roots,
            index_dir,
        } => {
            config::valid_name(&name)?;
            anyhow::ensure!(
                !cfg.collections.contains_key(&name),
                "collection {name:?} already exists"
            );
            let c = config::Collection {
                roots: roots
                    .iter()
                    .map(|p| config::Root {
                        path: root_path(p),
                        depth: None,
                    })
                    .collect(),
                index_dir,
                ..Default::default()
            };
            println!(
                "Created {name}; index: {}",
                config::index_dir(&name, &c)?.display()
            );
            cfg.collections.insert(name, c);
        }
        CollectionCmd::List => {
            for (n, c) in &cfg.collections {
                println!(
                    "{n}\t{} root(s)\t{}",
                    c.roots.len(),
                    config::index_dir(n, c)?.display()
                );
            }
            return Ok(());
        }
        CollectionCmd::Show { name } => {
            let c = cfg.get(&name)?;
            println!(
                "# {}\n# index_dir = {}",
                cfg_path.display(),
                config::index_dir(&name, c)?.display()
            );
            print!("{}", toml::to_string_pretty(c)?);
            return Ok(());
        }
        CollectionCmd::AddRoot {
            name,
            path,
            names_only,
        } => {
            let c = cfg.get_mut(&name)?;
            let path = root_path(&path);
            anyhow::ensure!(
                !c.roots.iter().any(|r| r.path == path),
                "root already present"
            );
            if !path.exists() {
                eprintln!(
                    "note: {} is not reachable now; it will be treated as offline until it is",
                    path.display()
                );
            }
            let depth = names_only.then(|| config::Depth {
                text: false,
                metadata: false,
                ..Default::default()
            });
            c.roots.push(config::Root { path, depth });
        }
        CollectionCmd::RemoveRoot { name, path } => {
            let c = cfg.get_mut(&name)?;
            let p = root_path(&path);
            let before = c.roots.len();
            c.roots.retain(|r| r.path != p && r.path != path);
            anyhow::ensure!(c.roots.len() < before, "no such root");
        }
        CollectionCmd::Exclude { name, glob } => {
            globset::Glob::new(&glob).context("invalid glob")?;
            cfg.get_mut(&name)?.exclude.push(glob);
        }
        CollectionCmd::Set { name, key, value } => {
            config::set_key(cfg.get_mut(&name)?, &key, &value)?
        }
        CollectionCmd::Remove { name } => {
            cfg.collections
                .remove(&name)
                .with_context(|| format!("no collection {name:?}"))?;
        }
        CollectionCmd::ConfigPath => {
            println!("{}", cfg_path.display());
            return Ok(());
        }
    }
    config::save(cfg_path, &cfg)
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1_048_576.0)
}

pub fn index(cfg_path: &Path, name: &str, dry_run: bool, json: bool) -> Result<()> {
    let cfg = load(cfg_path)?;
    let c = cfg.get(name)?;
    c.validate()?;
    let dir = config::index_dir(name, c)?;
    if !dry_run {
        let st = scan::run(c, &dir)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&st)?);
        } else {
            if st.resumed_after_interrupt {
                println!(
                    "Previous run was interrupted; unchanged work was kept and the rest resumed."
                );
            }
            println!(
                "Indexed {name}: {} files seen, {} unchanged, {} timestamp-only changes (not re-extracted), {} extracted, {} metadata-only, {} failed, {} marked missing, {} offline root(s)",
                st.files_seen, st.unchanged, st.rehashed_unchanged, st.extracted, st.metadata_only, st.failed, st.marked_missing, st.offline_roots
            );
            println!("Details: tpsearch status -c {name}");
        }
        return Ok(());
    }
    let inv = scan::inventory(c, &dir)?;
    let diff = scan::diff(&inv, &dir)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"inventory": inv, "new_changed_unchanged": diff})
            )?
        );
        return Ok(());
    }
    println!(
        "Dry run for {name} (nothing is written)\nIndex location: {}\n\nRoots:",
        dir.display()
    );
    for r in &inv.roots {
        let state = match (&r.duplicate_of, r.online) {
            (Some(d), _) => format!("DUPLICATE of {d} (skipped)"),
            (None, true) => "online".into(),
            (None, false) => format!("OFFLINE: {}", r.reason.as_deref().unwrap_or("")),
        };
        println!("  {}  [{state}]", r.path);
    }
    for o in &inv.overlaps {
        println!("  overlap: {o}");
    }
    let total: u64 = inv.by_ext.values().map(|x| x.0).sum();
    let bytes: u64 = inv.by_ext.values().map(|x| x.1).sum();
    println!(
        "\nFiles: {total} ({})\n  {:<10} {:>8} {:>12}  action",
        mb(bytes),
        "type",
        "files",
        "size"
    );
    let mut rows: Vec<_> = inv.by_ext.iter().collect();
    rows.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    for (ext, (n, b)) in rows {
        let ext_label = if ext.is_empty() { "(none)" } else { ext };
        println!(
            "  {ext_label:<10} {n:>8} {:>12}  {}",
            mb(*b),
            scan::ext_action(ext, c)
        );
    }
    let large = inv
        .candidates
        .iter()
        .filter(|x| x.size > c.max_file_size_mb * 1_048_576)
        .count();
    println!(
        "\nSkipped: {} excluded, {} hidden, {} symlinks/junctions, {} by extension filter, {} duplicate paths\nOver max_file_size_mb ({}): {large} (inventoried, not extracted)",
        inv.excluded, inv.hidden_skipped, inv.symlinks_skipped, inv.extension_skipped, inv.duplicate_paths_skipped, c.max_file_size_mb
    );
    for l in &inv.loops {
        println!("  traversal loop avoided at {l}");
    }
    for e in inv.errors.iter().take(20) {
        println!("  error: {e}");
    }
    if let Some((n, ch, same)) = diff {
        println!("Against current index: {n} new, {ch} changed, {same} unchanged");
    }
    Ok(())
}

pub fn search(cfg_path: &Path, a: SearchArgs) -> Result<()> {
    let cfg = load(cfg_path)?;
    let names: Vec<String> = if a.all {
        cfg.collections.keys().cloned().collect()
    } else {
        a.collection.clone()
    };
    anyhow::ensure!(
        !names.is_empty(),
        "pass -c COLLECTION (repeatable) or --all"
    );
    let opts = search::Opts {
        query: a.query.join(" "),
        phrase: a.phrase,
        any: a.any,
        ext: a.types.clone(),
        path: a.path.clone(),
        under: a.under.as_deref().map(report::resolve_folder),
        after: a.after.as_deref().map(db::parse_date).transpose()?,
        before: a.before.as_deref().map(db::parse_date).transpose()?,
        meta: a
            .meta
            .iter()
            .map(|m| {
                m.split_once('=')
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .context("--meta expects key=value")
            })
            .collect::<Result<_>>()?,
        representation: a.representation.clone(),
        limit: a.limit,
        include_missing: a.include_missing,
        verify: !a.no_verify,
    };
    let mut all = vec![];
    for n in &names {
        let c = cfg.get(n)?;
        all.push(search::run(n, &config::index_dir(n, c)?, &opts)?);
    }
    if a.json {
        let s = if all.len() == 1 {
            serde_json::to_string_pretty(&all[0])?
        } else {
            serde_json::to_string_pretty(&all)?
        };
        println!("{s}");
        return Ok(());
    }
    let tty = std::io::stdout().is_terminal();
    let (hl_on, hl_off) = if tty {
        ("\x1b[1;33m", "\x1b[0m")
    } else {
        ("[", "]")
    };
    for r in &all {
        let coll = r.hits.first().map(|h| h.collection.as_str()).unwrap_or("");
        println!(
            "{} matching passages in {} files = {} independent sources (copies and declared derived records count once){}",
            r.matching_passages, r.matching_files, r.independent_sources,
            if names.len() > 1 { format!(" [{coll}]") } else { String::new() }
        );
        println!("Relevance: {}\n", r.relevance_note);
        for (i, h) in r.hits.iter().enumerate() {
            let loc = match h.location.kind.as_str() {
                "metadata" => "metadata".to_string(),
                k => {
                    let mut s = if h.location.start == h.location.end {
                        format!("{k} {}", h.location.start)
                    } else {
                        format!("{k}s {}-{}", h.location.start, h.location.end)
                    };
                    if let (Some(a), Some(b)) = (&h.location.t_start, &h.location.t_end) {
                        s.push_str(&format!(", {a} --> {b}"));
                    }
                    if let Some(p) = h.location.page {
                        s.push_str(&format!(", page {p}"));
                    }
                    s
                }
            };
            println!(
                "{}. {} ({loc}){}",
                i + 1,
                h.path,
                h.location
                    .section
                    .as_ref()
                    .map(|s| format!(" § {s}"))
                    .unwrap_or_default()
            );
            let mut text: String = h
                .text
                .chars()
                .take(500)
                .collect::<String>()
                .replace('\n', " ");
            for t in &h.matched_terms {
                text = highlight(&text, t, hl_on, hl_off);
            }
            if let Some(b) = &h.context_before {
                println!("   … {b}");
            }
            println!("   > {text}");
            if let Some(x) = &h.context_after {
                println!("   … {x}");
            }
            let mut flags = vec![format!(
                "matched in {}: {}",
                h.matched_in,
                h.matched_terms.join(", ")
            )];
            flags.push(format!("relevance {:.2}", h.relevance));
            if h.representation != "original" {
                flags.push(h.representation.clone());
            }
            if h.root_offline {
                flags.push("ROOT OFFLINE".into());
            }
            if h.file_status == "missing" {
                flags.push("FILE MISSING".into());
            }
            if !h.quote_check.is_empty() {
                flags.push(h.quote_check.clone());
            }
            println!("   [{}]", flags.join(" | "));
            if !h.copies.is_empty() {
                println!(
                    "   identical copies (not counted separately): {}",
                    h.copies.join("; ")
                );
            }
            println!(
                "   open: tpsearch open -c {} {}   {}",
                h.collection, h.passage_id, h.uri
            );
        }
        if !r.file_matches.is_empty() {
            println!("\nFile name/path matches:");
            for f in &r.file_matches {
                println!(
                    "  {} [{}{}]",
                    f.path,
                    f.status,
                    if f.representation == "original" {
                        String::new()
                    } else {
                        format!(", {}", f.representation)
                    }
                );
            }
        }
    }
    Ok(())
}

fn highlight(text: &str, term: &str, on: &str, off: &str) -> String {
    let lt = text.to_lowercase();
    let lterm = term.to_lowercase();
    if lterm.is_empty() || lt.len() != text.len() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut i = 0;
    while let Some(j) = lt[i..].find(&lterm) {
        let s = i + j;
        out.push_str(&text[i..s]);
        out.push_str(on);
        out.push_str(&text[s..s + lterm.len()]);
        out.push_str(off);
        i = s + lterm.len();
    }
    out.push_str(&text[i..]);
    out
}

pub fn status(cfg_path: &Path, name: &str, json: bool) -> Result<()> {
    let cfg = load(cfg_path)?;
    let s = report::status(name, &config::index_dir(name, cfg.get(name)?)?)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(());
    }
    println!(
        "Collection {} (index {}){}",
        s.collection,
        s.index_dir,
        if s.paused { " — PAUSED" } else { "" }
    );
    println!("Roots:");
    for r in &s.roots {
        println!(
            "  {} [{}] {} files, last online {}",
            r["path"].as_str().unwrap_or(""),
            r["status"].as_str().unwrap_or(""),
            r["files"],
            r["last_online"].as_str().unwrap_or("never")
        );
    }
    println!(
        "Files by status: {}",
        s.files_by_status
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "Representations: {}",
        s.representations
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("Passages: {}", s.passages);
    println!("PDFs needing OCR: {}", s.pdf_needs_ocr);
    if !s.not_extracted_by_ext.is_empty() {
        println!("Not extracted (inventoried only):");
        for x in &s.not_extracted_by_ext {
            println!(
                "  .{} {} × {} — {}",
                x["ext"].as_str().unwrap_or(""),
                x["status"].as_str().unwrap_or(""),
                x["files"],
                x["reason"].as_str().unwrap_or("")
            );
        }
    }
    if !s.failures.is_empty() {
        println!("Extraction failures:");
        for f in &s.failures {
            println!(
                "  {} — {}",
                f["path"].as_str().unwrap_or(""),
                f["reason"].as_str().unwrap_or("")
            );
        }
    }
    println!("Recent runs:");
    for r in &s.recent_runs {
        println!(
            "  #{} {} {} → {}",
            r["run"],
            r["status"].as_str().unwrap_or(""),
            r["started"].as_str().unwrap_or(""),
            r["finished"].as_str().unwrap_or("-")
        );
    }
    Ok(())
}

pub fn pause(cfg_path: &Path, name: &str, on: bool) -> Result<()> {
    let cfg = load(cfg_path)?;
    let dir = config::index_dir(name, cfg.get(name)?)?;
    std::fs::create_dir_all(&dir)?;
    let flag = dir.join("PAUSE");
    if on {
        std::fs::write(&flag, b"paused\n")?;
        println!("Paused {name}; running workers wait between files.");
    } else {
        if flag.exists() {
            std::fs::remove_file(&flag)?;
        }
        println!("Resumed {name}.");
    }
    Ok(())
}

pub fn watch(cfg_path: &Path, name: &str, interval: Option<u64>) -> Result<()> {
    loop {
        let cfg = load(cfg_path)?;
        let c = cfg.get(name)?;
        let minutes = interval.unwrap_or(c.interval_minutes).max(1);
        match scan::run(c, &config::index_dir(name, c)?) {
            Ok(st) => eprintln!(
                "{}: {} extracted, {} unchanged, {} failed, {} missing, {} offline roots",
                db::iso(db::now()),
                st.extracted,
                st.unchanged,
                st.failed,
                st.marked_missing,
                st.offline_roots
            ),
            Err(e) => eprintln!("{}: index failed: {e:#}", db::iso(db::now())),
        }
        std::thread::sleep(Duration::from_secs(minutes * 60));
    }
}

fn viewer_program(program: &str) -> std::ffi::OsString {
    #[cfg(windows)]
    {
        // Windows editor launchers (including VS Code) may be .cmd files.
        let path = Path::new(program);
        if path.extension().is_none() {
            if let Some(paths) = std::env::var_os("PATH") {
                for dir in std::env::split_paths(&paths) {
                    for ext in ["exe", "com", "cmd", "bat"] {
                        let candidate = dir.join(program).with_extension(ext);
                        if candidate.is_file() {
                            return candidate.into_os_string();
                        }
                    }
                }
            }
        }
    }
    program.into()
}

pub fn open(cfg_path: &Path, name: &str, passage: i64, print: bool) -> Result<()> {
    let cfg = load(cfg_path)?;
    let c = cfg.get(name)?;
    let conn = db::open(&config::index_dir(name, c)?)?;
    let (path, kind, line, t): (String, String, i64, Option<String>) = conn
        .query_row(
            "SELECT f.path, p.loc_kind, p.loc_start, p.t_start FROM passages p JOIN files f ON f.id=p.file_id WHERE p.id=?1",
            [passage],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .with_context(|| format!("no passage {passage} in {name}"))?;
    let line = line.max(1);
    let loc = match (kind.as_str(), &t) {
        (_, Some(t)) => format!("{path}:{line} (at {t})"),
        ("paragraph", _) => format!("{path} (paragraph {line})"),
        _ => format!("{path}:{line}"),
    };
    if print {
        println!("{loc}");
        return Ok(());
    }
    anyhow::ensure!(
        Path::new(&path).exists(),
        "source is not reachable now: {path}"
    );
    let mut cmd = match &c.open_with {
        Some(tpl) => {
            let parts = shell_words::split(tpl)?;
            let parts: Vec<String> = parts
                .into_iter()
                .map(|p| {
                    p.replace("{path}", &path)
                        .replace("{line}", &line.to_string())
                })
                .collect();
            anyhow::ensure!(!parts.is_empty(), "open_with must name a program");
            let mut cmd = std::process::Command::new(viewer_program(&parts[0]));
            cmd.args(&parts[1..]);
            cmd
        }
        None if cfg!(windows) => {
            let mut cmd = std::process::Command::new("explorer.exe");
            cmd.arg(&path);
            cmd
        }
        None if cfg!(target_os = "macos") => {
            let mut cmd = std::process::Command::new("open");
            cmd.arg(&path);
            cmd
        }
        None => {
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(&path);
            cmd
        }
    };
    cmd.spawn().context("launching viewer")?;
    println!(
        "Opened {loc}{}",
        if c.open_with.is_none() {
            " (default app; set open_with to jump to the line)"
        } else {
            ""
        }
    );
    Ok(())
}

pub fn export_folder(
    cfg_path: &Path,
    name: &str,
    folder: &str,
    recursive: bool,
    out: Option<PathBuf>,
) -> Result<()> {
    let cfg = load(cfg_path)?;
    let c = cfg.get(name)?;
    let dir = config::index_dir(name, c)?;
    let folder = report::resolve_folder(folder);
    if let Some(o) = &out {
        let o = report::resolve_folder(&o.to_string_lossy());
        if c.roots
            .iter()
            .any(|r| Path::new(&o).starts_with(root_path(&r.path)))
        {
            eprintln!("note: output is inside an indexed root; it will be inventoried on the next index run");
        }
    }
    if recursive {
        let out = out.context("--recursive needs --out DIR")?;
        let n = report::folders_recursive(name, &dir, &folder, &out)?;
        println!(
            "Wrote {n} folder reports and manifest.json to {}",
            out.display()
        );
        return Ok(());
    }
    let v = report::folder(name, &dir, &folder)?;
    let s = serde_json::to_string_pretty(&v)?;
    match out {
        Some(p) => std::fs::write(&p, s).with_context(|| format!("writing {}", p.display()))?,
        None => println!("{s}"),
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    fn offline_unc_root_keeps_its_prefix() {
        let unc = Path::new(r"\\tpsearch-invalid-host\share\papers");
        assert_eq!(root_path(unc), unc);
        assert!(root_path(unc).is_absolute());
    }
}
