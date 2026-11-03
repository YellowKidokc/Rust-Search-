//! Inventory (walk) and incremental indexing of a collection.
//! Source files are opened read-only; nothing is written beneath a root.
use crate::{
    config::{Collection, Depth},
    db,
    extract::{self, Support},
};
use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant, UNIX_EPOCH},
};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct RootState {
    pub configured: String,
    pub path: String,
    pub online: bool,
    pub reason: Option<String>,
    /// Same directory as an earlier root; skipped to avoid duplicate ingestion.
    pub duplicate_of: Option<String>,
    #[serde(skip)]
    pub depth: Depth,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub root: usize,
    pub path: PathBuf,
    pub key: String,
    pub rel: String,
    pub name: String,
    pub ext: String,
    pub size: u64,
    pub mtime: i64,
    pub mtime_ns: i64,
}

#[derive(Debug, Default, Serialize)]
pub struct Inventory {
    pub roots: Vec<RootState>,
    pub overlaps: Vec<String>,
    pub by_ext: BTreeMap<String, (u64, u64)>,
    pub excluded: u64,
    pub hidden_skipped: u64,
    pub symlinks_skipped: u64,
    pub extension_skipped: u64,
    pub duplicate_paths_skipped: u64,
    pub loops: Vec<String>,
    pub errors: Vec<String>,
    #[serde(skip)]
    pub candidates: Vec<Candidate>,
}

fn key_of(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn globs(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(
            GlobBuilder::new(p)
                .case_insensitive(true)
                .literal_separator(false)
                .build()
                .with_context(|| format!("invalid exclude glob {p:?}"))?,
        );
    }
    Ok(b.build()?)
}

#[cfg(windows)]
fn hidden_attr(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x2 != 0
}
#[cfg(not(windows))]
fn hidden_attr(_: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn reparse_point(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn reparse_point(_: &fs::Metadata) -> bool {
    false
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || reparse_point(&m))
}

fn root_state(cfg: &Collection) -> Vec<RootState> {
    let mut out: Vec<RootState> = vec![];
    for r in &cfg.roots {
        let configured = r.path.to_string_lossy().into_owned();
        let probe = fs::metadata(&r.path).and_then(|m| {
            if m.is_dir() {
                fs::read_dir(&r.path).map(|_| ())
            } else {
                Ok(())
            }
        });
        let canon = dunce::canonicalize(&r.path).unwrap_or_else(|_| r.path.clone());
        let path = key_of(&canon);
        let duplicate_of = out
            .iter()
            .find(|o| o.path == path)
            .map(|o| o.configured.clone());
        out.push(RootState {
            configured,
            path,
            online: probe.is_ok(),
            reason: probe.err().map(|e| e.to_string()),
            duplicate_of,
            depth: cfg.depth_for(r),
        });
    }
    out
}

/// Walk all online roots. No hashing, no reads of file contents.
pub fn inventory(cfg: &Collection, index_dir: &Path) -> Result<Inventory> {
    let mut inv = Inventory {
        roots: root_state(cfg),
        ..Default::default()
    };
    let excludes = globs(&cfg.exclude)?;
    let index_canon = dunce::canonicalize(index_dir).unwrap_or_else(|_| index_dir.to_path_buf());
    let active: Vec<usize> = (0..inv.roots.len())
        .filter(|&i| inv.roots[i].online && inv.roots[i].duplicate_of.is_none())
        .collect();
    for &i in &active {
        for &j in &active {
            let (a, b) = (Path::new(&inv.roots[i].path), Path::new(&inv.roots[j].path));
            if i != j && b.starts_with(a) {
                inv.overlaps.push(format!(
                    "{} is inside {}; its files are indexed once, under the inner root's settings",
                    inv.roots[j].path, inv.roots[i].path
                ));
            }
        }
        if index_canon.starts_with(&inv.roots[i].path) {
            inv.overlaps.push(format!(
                "index_dir {} is inside root {}; it is excluded",
                index_canon.display(),
                inv.roots[i].path
            ));
        }
    }
    let exts: HashSet<&str> = cfg.extensions.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    for &i in &active {
        let root = PathBuf::from(&inv.roots[i].path);
        let nested: HashSet<PathBuf> = active
            .iter()
            .filter(|&&j| j != i && Path::new(&inv.roots[j].path).starts_with(&root))
            .map(|&j| PathBuf::from(&inv.roots[j].path))
            .collect();
        let rel_of = |p: &Path| -> String {
            p.strip_prefix(&root)
                .unwrap_or(p)
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        };
        let mut skipped = (0u64, 0u64, 0u64);
        let mut visited_dirs = std::collections::HashSet::from([
            dunce::canonicalize(&root).unwrap_or_else(|_| root.clone())
        ]);
        let mut junction_loops = Vec::new();
        let walker = WalkDir::new(&root)
            .follow_links(cfg.follow_symlinks)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let p = e.path();
                if !cfg.follow_symlinks && is_link(p) {
                    skipped.2 += 1;
                    return false;
                }
                if e.file_type().is_dir() && (nested.contains(p) || p == index_canon) {
                    return false;
                }
                let rel = rel_of(p);
                let excluded = excludes.is_match(&rel)
                    || (e.file_type().is_dir() && excludes.is_match(format!("{rel}/_")));
                let hidden = !cfg.include_hidden
                    && (e.file_name().to_string_lossy().starts_with('.')
                        || e.metadata().is_ok_and(|m| hidden_attr(&m)));
                if excluded {
                    skipped.0 += 1;
                } else if hidden {
                    skipped.1 += 1;
                }
                if excluded || hidden {
                    return false;
                }
                // Junctions can appear as directories rather than symbolic links.
                // Canonical directory identities prevent revisiting either kind.
                if cfg.follow_symlinks && e.file_type().is_dir() {
                    if let Ok(canonical) = dunce::canonicalize(p) {
                        if !visited_dirs.insert(canonical) {
                            junction_loops.push(p.display().to_string());
                            return false;
                        }
                    }
                }
                true
            });
        for entry in walker {
            let e = match entry {
                Ok(e) => e,
                Err(err) => {
                    let at = err
                        .path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    if err.loop_ancestor().is_some() {
                        inv.loops.push(at);
                    } else {
                        inv.errors.push(format!("{at}: {err}"));
                    }
                    continue;
                }
            };
            if is_link(e.path()) && !cfg.follow_symlinks {
                inv.symlinks_skipped += 1;
                continue;
            }
            if !e.file_type().is_file() {
                continue;
            }
            let path = if cfg.follow_symlinks {
                dunce::canonicalize(e.path()).unwrap_or_else(|_| e.path().to_path_buf())
            } else {
                e.path().to_path_buf()
            };
            let key = key_of(&path);
            if !seen.insert(key.clone()) {
                inv.duplicate_paths_skipped += 1;
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let ext = Path::new(&name)
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !exts.is_empty() && !exts.contains(ext.as_str()) {
                inv.extension_skipped += 1;
                continue;
            }
            let m = match e.metadata() {
                Ok(m) => m,
                Err(err) => {
                    inv.errors.push(format!("{key}: {err}"));
                    continue;
                }
            };
            let since = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .unwrap_or_default();
            let entry = inv.by_ext.entry(ext.clone()).or_default();
            entry.0 += 1;
            entry.1 += m.len();
            inv.candidates.push(Candidate {
                root: i,
                rel: rel_of(e.path()),
                path,
                key,
                name,
                ext,
                size: m.len(),
                mtime: since.as_secs() as i64,
                mtime_ns: since.subsec_nanos() as i64,
            });
        }
        inv.excluded += skipped.0;
        inv.hidden_skipped += skipped.1;
        inv.symlinks_skipped += skipped.2;
        inv.loops.extend(junction_loops);
    }
    Ok(inv)
}

/// What the current settings want for a file: (extractor, version, status-if-not-extracted, reason).
fn plan(
    c: &Candidate,
    depth: &Depth,
    cfg: &Collection,
) -> (String, i64, Option<(&'static str, String)>) {
    if !depth.text {
        return (
            "none".into(),
            0,
            Some((
                "metadata_only",
                "text extraction disabled for this root".into(),
            )),
        );
    }
    match extract::support(&c.ext) {
        Support::Adapter(a) if c.size > cfg.max_file_size_mb * 1024 * 1024 => (
            a.name.into(),
            a.version,
            Some((
                "too_large",
                format!("larger than max_file_size_mb = {}", cfg.max_file_size_mb),
            )),
        ),
        Support::Adapter(a) => (a.name.into(), a.version, None),
        Support::Pending(why) => ("pending".into(), 0, Some(("pending_adapter", why.into()))),
        Support::Unsupported => (
            "none".into(),
            0,
            Some(("metadata_only", format!("no text adapter for .{}", c.ext))),
        ),
    }
}

#[derive(Debug, Default, Serialize, Clone)]
pub struct Stats {
    pub files_seen: u64,
    pub unchanged: u64,
    /// Timestamp changed but content hash identical: no re-extraction.
    pub rehashed_unchanged: u64,
    pub extracted: u64,
    pub metadata_only: u64,
    pub failed: u64,
    pub marked_missing: u64,
    pub offline_roots: u64,
    pub resumed_after_interrupt: bool,
}

struct Prior {
    id: i64,
    size: i64,
    mtime: i64,
    mtime_ns: i64,
    sha: Option<String>,
    status: String,
    extractor: Option<String>,
    version: Option<i64>,
}

enum Outcome {
    Same,
    Extracted {
        sha: Option<String>,
        ex: extract::Extracted,
        adapter: &'static extract::Adapter,
    },
    NotExtracted {
        sha: Option<String>,
        status: &'static str,
        reason: String,
    },
    Failed {
        sha: Option<String>,
        reason: String,
    },
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn process(c: &Candidate, depth: &Depth, cfg: &Collection, prior: Option<&Prior>) -> Outcome {
    let (extractor, version, skip) = plan(c, depth, cfg);
    let same_plan = |p: &Prior| {
        p.extractor.as_deref() == Some(extractor.as_str()) && p.version == Some(version)
    };
    let hash_ok = c.size <= cfg.hash_max_size_mb * 1024 * 1024;
    let adapter = match extract::support(&c.ext) {
        Support::Adapter(a) if skip.is_none() => Some(a),
        _ => None,
    };
    let (sha, bytes) = match adapter {
        Some(_) => match fs::read(&c.path) {
            Ok(b) => (Some(format!("{:x}", Sha256::digest(&b))), Some(b)),
            Err(e) => {
                return Outcome::Failed {
                    sha: None,
                    reason: format!("read failed: {e}"),
                }
            }
        },
        None if hash_ok => match hash_file(&c.path) {
            Ok(h) => (Some(h), None),
            Err(e) => {
                return Outcome::Failed {
                    sha: None,
                    reason: format!("read failed: {e}"),
                }
            }
        },
        None => (None, None),
    };
    if let Some(p) = prior {
        if p.sha.is_some() && p.sha == sha && same_plan(p) && p.status != "missing" {
            return Outcome::Same;
        }
    }
    if let Some((status, reason)) = skip {
        return Outcome::NotExtracted {
            sha,
            status,
            reason,
        };
    }
    let a = adapter.expect("adapter present when not skipped");
    match a.extract(bytes.as_deref().unwrap_or_default(), &c.path) {
        Ok(ex) => Outcome::Extracted {
            sha,
            ex,
            adapter: a,
        },
        Err(e) => Outcome::Failed {
            sha,
            reason: format!("{} adapter: {e:#}", a.name),
        },
    }
}

fn wait_if_paused(index_dir: &Path) {
    let flag = index_dir.join("PAUSE");
    let mut told = false;
    while flag.exists() {
        if !told {
            eprintln!("paused (remove with `tpsearch resume <collection>`)");
            told = true;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

pub fn run(cfg: &Collection, index_dir: &Path) -> Result<Stats> {
    cfg.validate()?;
    let mut conn = db::open(index_dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(index_dir.join("index.lock"))?;
    lock.try_lock()
        .map_err(|_| anyhow::anyhow!("another indexing run is active for this collection"))?;
    let mut st = Stats::default();
    st.resumed_after_interrupt = conn.execute(
        "UPDATE runs SET status='interrupted' WHERE status='running'",
        [],
    )? > 0;
    let now = db::now();
    conn.execute(
        "INSERT INTO runs(started,status) VALUES(?1,'running')",
        [now],
    )?;
    let run_id = conn.last_insert_rowid();

    let inv = inventory(cfg, index_dir)?;
    let mut root_ids = vec![];
    for r in &inv.roots {
        conn.execute(
            "INSERT INTO roots(path,status,last_checked,last_online) VALUES(?1,?2,?3,CASE WHEN ?2='online' THEN ?3 END)
             ON CONFLICT(path) DO UPDATE SET status=?2, last_checked=?3, last_online=COALESCE(CASE WHEN ?2='online' THEN ?3 END, last_online)",
            params![r.path, if r.online { "online" } else { "offline" }, now],
        )?;
        root_ids.push(
            conn.query_row("SELECT id FROM roots WHERE path=?1", [&r.path], |x| {
                x.get::<_, i64>(0)
            })?,
        );
        if !r.online {
            st.offline_roots += 1;
            eprintln!(
                "offline: {} ({}); its indexed records are kept",
                r.configured,
                r.reason.as_deref().unwrap_or("unavailable")
            );
        }
    }
    let mut prior: HashMap<String, Prior> = HashMap::new();
    {
        let mut q = conn.prepare("SELECT id,path,size,mtime,mtime_ns,sha256,status,extractor,extractor_version FROM files")?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                Prior {
                    id: r.get(0)?,
                    size: r.get(2)?,
                    mtime: r.get(3)?,
                    mtime_ns: r.get(4)?,
                    sha: r.get(5)?,
                    status: r.get(6)?,
                    extractor: r.get(7)?,
                    version: r.get(8)?,
                },
            ))
        })?;
        for x in rows {
            let (k, p) = x?;
            prior.insert(k, p);
        }
    }
    st.files_seen = inv.candidates.len() as u64;
    let mut work = vec![];
    {
        let tx = conn.transaction()?;
        {
            let mut touch =
                tx.prepare("UPDATE files SET last_seen_run=?1, root_id=?2 WHERE id=?3")?;
            for c in &inv.candidates {
                let depth = &inv.roots[c.root].depth;
                let (extractor, version, _) = plan(c, depth, cfg);
                match prior.get(&c.key) {
                    Some(p)
                        if p.size == c.size as i64
                            && p.mtime == c.mtime
                            && p.mtime_ns == c.mtime_ns
                            && p.extractor.as_deref() == Some(extractor.as_str())
                            && p.version == Some(version)
                            && p.status != "missing" =>
                    {
                        touch.execute(params![run_id, root_ids[c.root], p.id])?;
                        st.unchanged += 1;
                    }
                    _ => work.push(c.clone()),
                }
            }
        }
        tx.commit()?;
    }

    let total = work.len();
    let queue = Arc::new(Mutex::new(work.into_iter()));
    let prior = Arc::new(prior);
    let (send, recv) = mpsc::sync_channel::<(Candidate, Outcome)>(64);
    std::thread::scope(|s| -> Result<()> {
        for _ in 0..cfg.threads {
            let (queue, prior, send) = (queue.clone(), prior.clone(), send.clone());
            let roots = &inv.roots;
            s.spawn(move || loop {
                wait_if_paused(index_dir);
                let Some(c) = queue.lock().unwrap().next() else {
                    break;
                };
                let out = process(&c, &roots[c.root].depth, cfg, prior.get(&c.key));
                if send.send((c, out)).is_err() {
                    break;
                }
                if cfg.throttle_ms > 0 {
                    std::thread::sleep(Duration::from_millis(cfg.throttle_ms));
                }
            });
        }
        drop(send);
        let mut done = 0usize;
        let mut last = Instant::now();
        let mut tx = conn.transaction()?;
        for (c, out) in recv {
            done += 1;
            let depth = &inv.roots[c.root].depth;
            write(
                &tx,
                &c,
                root_ids[c.root],
                run_id,
                out,
                prior.get(&c.key),
                depth,
                cfg,
                &mut st,
            )?;
            if done.is_multiple_of(200) {
                tx.commit()?;
                tx = conn.transaction()?;
            }
            if last.elapsed() > Duration::from_secs(2) {
                eprintln!("[{done}/{total}] {}", c.key);
                last = Instant::now();
            }
        }
        tx.commit()?;
        Ok(())
    })?;

    let online: Vec<i64> = inv
        .roots
        .iter()
        .zip(&root_ids)
        .filter(|(r, _)| r.online && r.duplicate_of.is_none())
        .map(|(_, id)| *id)
        .collect();
    for id in online {
        st.marked_missing += conn.execute(
            "UPDATE files SET status='missing', status_reason='not found during last scan of an online root'
             WHERE root_id=?1 AND last_seen_run<>?2 AND status<>'missing'",
            params![id, run_id],
        )? as u64;
    }
    relate(&conn)?;
    conn.execute(
        "UPDATE runs SET finished=?1, status='complete', stats=?2 WHERE id=?3",
        params![db::now(), serde_json::to_string(&st)?, run_id],
    )?;
    Ok(st)
}

#[allow(clippy::too_many_arguments)]
fn write(
    tx: &Connection,
    c: &Candidate,
    root_id: i64,
    run: i64,
    out: Outcome,
    prior: Option<&Prior>,
    depth: &Depth,
    cfg: &Collection,
    st: &mut Stats,
) -> Result<()> {
    let (extractor, version, _) = plan(c, depth, cfg);
    let dir = c.path.parent().map(key_of).unwrap_or_default();
    let upsert = |sha: &Option<String>,
                  status: &str,
                  reason: Option<&str>,
                  extractor: Option<(&str, i64)>,
                  n: usize,
                  derived: Option<&str>|
     -> Result<i64> {
        Ok(tx.query_row(
            "INSERT INTO files(root_id,path,dir,rel_path,name,ext,size,mtime,mtime_ns,sha256,status,status_reason,extractor,extractor_version,extracted_at,passage_count,derived_from,first_seen,last_seen_run)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?15,?18)
             ON CONFLICT(path) DO UPDATE SET root_id=?1,dir=?3,rel_path=?4,name=?5,ext=?6,size=?7,mtime=?8,mtime_ns=?9,sha256=?10,status=?11,status_reason=?12,
               extractor=?13,extractor_version=?14,extracted_at=?15,passage_count=?16,derived_from=?17,last_seen_run=?18
             RETURNING id",
            params![root_id, c.key, dir, c.rel, c.name, c.ext, c.size as i64, c.mtime, c.mtime_ns, sha, status, reason,
                    extractor.map(|e| e.0), extractor.map(|e| e.1), db::now(), n as i64, derived, run],
            |r| r.get(0),
        )?)
    };
    let clear = |id: i64| -> Result<()> {
        tx.execute(
            "DELETE FROM passages_fts WHERE rowid IN (SELECT id FROM passages WHERE file_id=?1)",
            [id],
        )?;
        tx.execute("DELETE FROM passages WHERE file_id=?1", [id])?;
        tx.execute("DELETE FROM meta WHERE file_id=?1", [id])?;
        tx.execute("DELETE FROM names_fts WHERE rowid=?1", [id])?;
        tx.execute(
            "INSERT INTO names_fts(rowid,name,rel_path) VALUES(?1,?2,?3)",
            params![id, c.name, c.rel],
        )?;
        Ok(())
    };
    match out {
        Outcome::Same => {
            let p = prior.expect("Same implies prior");
            tx.execute(
                "UPDATE files SET size=?1,mtime=?2,mtime_ns=?3,last_seen_run=?4,root_id=?5 WHERE id=?6",
                params![c.size as i64, c.mtime, c.mtime_ns, run, root_id, p.id],
            )?;
            st.rehashed_unchanged += 1;
        }
        Outcome::NotExtracted {
            sha,
            status,
            reason,
        } => {
            let id = upsert(
                &sha,
                status,
                Some(&reason),
                Some((&extractor, version)),
                0,
                None,
            )?;
            clear(id)?;
            st.metadata_only += 1;
        }
        Outcome::Failed { sha, reason } => {
            // Recording the plan means an unchanged failing file is not retried every run.
            let id = upsert(
                &sha,
                "failed",
                Some(&reason),
                Some((&extractor, version)),
                0,
                None,
            )?;
            clear(id)?;
            st.failed += 1;
        }
        Outcome::Extracted { sha, ex, adapter } => {
            let n = ex.passages.len();
            let id = upsert(
                &sha,
                "indexed",
                None,
                Some((adapter.name, adapter.version)),
                n,
                ex.derived_from.as_deref(),
            )?;
            clear(id)?;
            let mut ins = tx.prepare_cached(
                "INSERT INTO passages(file_id,ord,loc_kind,loc_start,loc_end,section,t_start,t_end,page) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            )?;
            let mut fts =
                tx.prepare_cached("INSERT INTO passages_fts(rowid,text,section) VALUES(?1,?2,?3)")?;
            for (i, p) in ex.passages.iter().enumerate() {
                ins.execute(params![
                    id,
                    i as i64,
                    adapter.loc.as_str(),
                    p.loc_start,
                    p.loc_end,
                    p.section,
                    p.t_start,
                    p.t_end,
                    p.page
                ])?;
                fts.execute(params![tx.last_insert_rowid(), p.text, p.section])?;
            }
            if depth.metadata && !ex.meta.is_empty() {
                let mut m =
                    tx.prepare_cached("INSERT INTO meta(file_id,key,value) VALUES(?1,?2,?3)")?;
                for (k, v) in &ex.meta {
                    m.execute(params![id, k, v])?;
                }
                let text = ex
                    .meta
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                ins.execute(params![
                    id,
                    -1i64,
                    "metadata",
                    0,
                    0,
                    "[metadata]",
                    None::<String>,
                    None::<String>,
                    None::<i64>
                ])?;
                fts.execute(params![tx.last_insert_rowid(), text, "[metadata]"])?;
            }
            st.extracted += 1;
        }
    }
    Ok(())
}

/// Recompute copy / transcript / derived relationships. "Original" is the
/// earliest-indexed member of a byte-identical group; this is a bookkeeping
/// choice, not a claim about which file was created first.
fn relate(c: &Connection) -> Result<()> {
    c.execute_batch(
        "UPDATE files SET copy_of=NULL, representation=CASE
            WHEN ext IN ('srt','vtt') THEN 'transcript'
            WHEN derived_from IS NOT NULL THEN 'derived'
            ELSE 'original' END;
         UPDATE files SET
            copy_of=(SELECT MIN(f2.id) FROM files f2 WHERE f2.sha256=files.sha256 AND f2.status<>'missing' AND f2.size>0),
            representation='copy'
         WHERE sha256 IS NOT NULL AND status<>'missing' AND size>0
           AND id > (SELECT MIN(f2.id) FROM files f2 WHERE f2.sha256=files.sha256 AND f2.status<>'missing' AND f2.size>0);
         UPDATE files SET derived_from_id=(
            SELECT f2.id FROM files f2 WHERE f2.id<>files.id AND (f2.sha256=lower(files.derived_from)
               OR f2.path=files.derived_from OR f2.rel_path=replace(files.derived_from,'\\','/') OR f2.name=files.derived_from)
            ORDER BY f2.id LIMIT 1)
         WHERE derived_from IS NOT NULL;",
    )?;
    Ok(())
}

/// For a dry run: how many inventoried files are new / changed / unchanged versus the index.
pub fn diff(inv: &Inventory, index_dir: &Path) -> Result<Option<(u64, u64, u64)>> {
    if !db::exists(index_dir) {
        return Ok(None);
    }
    let c = db::open(index_dir)?;
    let mut q = c.prepare("SELECT size,mtime,mtime_ns FROM files WHERE path=?1")?;
    let (mut new, mut changed, mut same) = (0, 0, 0);
    for x in &inv.candidates {
        match q
            .query_row([&x.key], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .optional()?
        {
            None => new += 1,
            Some(s) if s == (x.size as i64, x.mtime, x.mtime_ns) => same += 1,
            Some(_) => changed += 1,
        }
    }
    Ok(Some((new, changed, same)))
}

pub fn ext_action(ext: &str, cfg: &Collection) -> &'static str {
    if !cfg.depth.text {
        return "metadata only (text disabled)";
    }
    match extract::support(ext) {
        Support::Adapter(_) => "extract text",
        Support::Pending(_) => "pending adapter (metadata only)",
        Support::Unsupported => "metadata only",
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn windows_hidden_attribute_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dotfile.txt");
        fs::write(&file, "hidden by Windows attribute").unwrap();
        assert!(!hidden_attr(&fs::metadata(&file).unwrap()));
        assert!(Command::new("attrib")
            .arg("+H")
            .arg(&file)
            .status()
            .unwrap()
            .success());
        assert!(hidden_attr(&fs::metadata(&file).unwrap()));
        assert!(Command::new("attrib")
            .arg("-H")
            .arg(&file)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn junction_detection_and_loop_prevention() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("docs");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("note.txt"), "junction test").unwrap();
        let junction = root.join("loop");
        assert!(Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&root)
            .status()
            .unwrap()
            .success());
        assert!(is_link(&junction));
        assert!(reparse_point(&fs::symlink_metadata(&junction).unwrap()));
        let mut cfg = Collection {
            roots: vec![crate::config::Root {
                path: root,
                depth: None,
            }],
            ..Default::default()
        };
        let inventory_default = inventory(&cfg, &tmp.path().join("index")).unwrap();
        assert_eq!(inventory_default.candidates.len(), 1);
        assert!(inventory_default.symlinks_skipped > 0);
        cfg.follow_symlinks = true;
        let followed = inventory(&cfg, &tmp.path().join("index")).unwrap();
        assert_eq!(followed.candidates.len(), 1);
        assert!(!followed.loops.is_empty());
        // Remove only the junction itself, never traverse it during test cleanup.
        fs::remove_dir(junction).unwrap();
    }
}
