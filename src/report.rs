//! Read-only reports over an index: collection status, and the folder-report
//! JSON export (`tpsearch.folder_report/v1`, see docs/FOLDER_REPORT_EXPORT.md).
use crate::{db, search::file_uri};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const FOLDER_SCHEMA: &str = "tpsearch.folder_report/v1";

pub fn stable_id(prefix: &str, path: &str) -> String {
    let h = format!("{:x}", Sha256::digest(path.as_bytes()));
    format!("{prefix}-{}", &h[..12])
}

#[derive(Serialize)]
pub struct Status {
    pub collection: String,
    pub index_dir: String,
    pub roots: Vec<Value>,
    pub files_by_status: BTreeMap<String, i64>,
    pub representations: BTreeMap<String, i64>,
    pub passages: i64,
    pub pdf_needs_ocr: i64,
    pub not_extracted_by_ext: Vec<Value>,
    pub failures: Vec<Value>,
    pub recent_runs: Vec<Value>,
    pub paused: bool,
}

fn group(c: &Connection, sql: &str) -> Result<BTreeMap<String, i64>> {
    let mut q = c.prepare(sql)?;
    let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn status(collection: &str, index_dir: &Path) -> Result<Status> {
    anyhow::ensure!(
        db::exists(index_dir),
        "collection {collection:?} has not been indexed yet"
    );
    let c = db::open(index_dir)?;
    let mut q = c.prepare(
        "SELECT r.path, r.status, r.last_checked, r.last_online, COUNT(f.id) FROM roots r
         LEFT JOIN files f ON f.root_id=r.id AND f.status<>'missing' GROUP BY r.id ORDER BY r.id",
    )?;
    let roots = q
        .query_map([], |r| {
            Ok(json!({"path": r.get::<_, String>(0)?, "status": r.get::<_, String>(1)?,
                "last_checked": r.get::<_, Option<i64>>(2)?.map(db::iso), "last_online": r.get::<_, Option<i64>>(3)?.map(db::iso),
                "files": r.get::<_, i64>(4)?}))
        })?
        .collect::<Result<_, _>>()?;
    let mut q = c.prepare(
        "SELECT ext, status, COUNT(*), MIN(status_reason) FROM files WHERE status IN ('metadata_only','pending_adapter','too_large')
         GROUP BY ext, status ORDER BY COUNT(*) DESC LIMIT 30",
    )?;
    let not_extracted_by_ext = q
        .query_map([], |r| {
            Ok(json!({"ext": r.get::<_, String>(0)?, "status": r.get::<_, String>(1)?, "files": r.get::<_, i64>(2)?, "reason": r.get::<_, Option<String>>(3)?}))
        })?
        .collect::<Result<_, _>>()?;
    let mut q = c.prepare(
        "SELECT path, status_reason FROM files WHERE status='failed' ORDER BY path LIMIT 50",
    )?;
    let failures = q
        .query_map([], |r| {
            Ok(json!({"path": r.get::<_, String>(0)?, "reason": r.get::<_, Option<String>>(1)?}))
        })?
        .collect::<Result<_, _>>()?;
    let mut q = c.prepare(
        "SELECT id, started, finished, status, stats FROM runs ORDER BY id DESC LIMIT 5",
    )?;
    let recent_runs = q
        .query_map([], |r| {
            let stats: Option<String> = r.get(4)?;
            Ok(json!({"run": r.get::<_, i64>(0)?, "started": db::iso(r.get(1)?), "finished": r.get::<_, Option<i64>>(2)?.map(db::iso),
                "status": r.get::<_, String>(3)?, "stats": stats.and_then(|s| serde_json::from_str::<Value>(&s).ok())}))
        })?
        .collect::<Result<_, _>>()?;
    Ok(Status {
        collection: collection.into(),
        index_dir: index_dir.display().to_string(),
        roots,
        files_by_status: group(&c, "SELECT status, COUNT(*) FROM files GROUP BY status")?,
        representations: group(&c, "SELECT representation, COUNT(*) FROM files WHERE status<>'missing' GROUP BY representation")?,
        passages: c.query_row("SELECT COUNT(*) FROM passages WHERE ord>=0", [], |r| r.get(0))?,
        pdf_needs_ocr: c.query_row("SELECT COUNT(DISTINCT file_id) FROM meta WHERE key='pdf_type' AND value='image_only'", [], |r| r.get(0))?,
        not_extracted_by_ext,
        failures,
        recent_runs,
        paused: index_dir.join("PAUSE").exists(),
    })
}

fn file_json(c: &Connection, id: i64) -> Result<Value> {
    let mut v: Map<String, Value> = c.query_row(
        "SELECT f.id, f.name, f.path, f.rel_path, f.ext, f.size, f.mtime, f.sha256, f.status, f.status_reason, f.extractor,
                f.extractor_version, f.passage_count, f.representation, f.copy_of, f.derived_from, f.derived_from_id, f.first_seen,
                c.path, d.path
         FROM files f LEFT JOIN files c ON c.id=f.copy_of LEFT JOIN files d ON d.id=f.derived_from_id WHERE f.id=?1",
        [id],
        |r| {
            let path: String = r.get(2)?;
            let mut m = Map::new();
            m.insert("file_id".into(), json!(r.get::<_, i64>(0)?));
            m.insert("stable_id".into(), json!(stable_id("FIL", &path)));
            m.insert("name".into(), json!(r.get::<_, String>(1)?));
            m.insert("uri".into(), json!(file_uri(&path)));
            m.insert("path".into(), json!(path));
            m.insert("rel_path".into(), json!(r.get::<_, String>(3)?));
            m.insert("ext".into(), json!(r.get::<_, String>(4)?));
            m.insert("size".into(), json!(r.get::<_, i64>(5)?));
            m.insert("modified".into(), json!(db::iso(r.get(6)?)));
            m.insert("sha256".into(), json!(r.get::<_, Option<String>>(7)?));
            m.insert("status".into(), json!(r.get::<_, String>(8)?));
            m.insert("status_reason".into(), json!(r.get::<_, Option<String>>(9)?));
            m.insert("extractor".into(), json!({"name": r.get::<_, Option<String>>(10)?, "version": r.get::<_, Option<i64>>(11)?}));
            m.insert("passage_count".into(), json!(r.get::<_, i64>(12)?));
            m.insert("representation".into(), json!(r.get::<_, String>(13)?));
            m.insert("copy_of".into(), match r.get::<_, Option<i64>>(14)? {
                Some(cid) => json!({"file_id": cid, "path": r.get::<_, Option<String>>(18)?}),
                None => Value::Null,
            });
            m.insert("derived_from".into(), match r.get::<_, Option<String>>(15)? {
                Some(decl) => json!({"declared": decl, "file_id": r.get::<_, Option<i64>>(16)?, "path": r.get::<_, Option<String>>(19)?}),
                None => Value::Null,
            });
            m.insert("first_indexed".into(), json!(db::iso(r.get(17)?)));
            Ok(m)
        },
    )?;
    let mut q = c.prepare_cached("SELECT key, value FROM meta WHERE file_id=?1 ORDER BY rowid")?;
    let mut meta: Map<String, Value> = Map::new();
    for kv in q.query_map([id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (k, val) = kv?;
        meta.entry(k)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!(val));
    }
    v.insert(
        "metadata".into(),
        json!({"origin": "source", "fields": meta}),
    );
    v.insert("classifications".into(), json!([]));
    Ok(Value::Object(v))
}

fn last_run(c: &Connection) -> Result<Value> {
    Ok(c.query_row("SELECT id, started, finished, status FROM runs ORDER BY id DESC LIMIT 1", [], |r| {
        Ok(json!({"run": r.get::<_, i64>(0)?, "started": db::iso(r.get(1)?), "finished": r.get::<_, Option<i64>>(2)?.map(db::iso), "status": r.get::<_, String>(3)?}))
    }).optional()?.unwrap_or(Value::Null))
}

pub fn folder(collection: &str, index_dir: &Path, folder: &str) -> Result<Value> {
    anyhow::ensure!(
        db::exists(index_dir),
        "collection {collection:?} has not been indexed yet"
    );
    let c = db::open(index_dir)?;
    folder_with(&c, collection, folder)
}

fn folder_with(c: &Connection, collection: &str, folder: &str) -> Result<Value> {
    let folder = folder.trim_end_matches(['/', '\\']).to_string();
    let sep = if folder.contains('\\') || folder.starts_with(r"\\") {
        '\\'
    } else {
        '/'
    };
    let prefix = format!("{folder}{sep}");
    let root: Option<(String, String)> = c
        .query_row(
            "SELECT path, status FROM roots WHERE ?1=path OR substr(?1,1,length(path)+1) IN (path||'/', path||'\\') ORDER BY length(path) DESC LIMIT 1",
            [&folder],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (root, root_status) = root
        .with_context(|| format!("{folder} is not inside any root of collection {collection:?}"))?;
    let mut q =
        c.prepare("SELECT id FROM files WHERE dir=?1 AND status<>'missing' ORDER BY name")?;
    let ids: Vec<i64> = q
        .query_map([&folder], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let files: Vec<Value> = ids
        .iter()
        .map(|&id| file_json(c, id))
        .collect::<Result<_>>()?;
    let missing: i64 = c.query_row(
        "SELECT COUNT(*) FROM files WHERE dir=?1 AND status='missing'",
        [&folder],
        |r| r.get(0),
    )?;
    // Child folders: first path component below this folder, from indexed file dirs.
    let mut q = c.prepare("SELECT dir, COUNT(*) FROM files WHERE substr(dir,1,length(?1))=?1 AND status<>'missing' GROUP BY dir")?;
    let mut children: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for row in q.query_map([&prefix], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })? {
        let (dir, n) = row?;
        let child = dir[prefix.len()..].split(sep).next().unwrap_or_default();
        let e = children.entry(format!("{prefix}{child}")).or_default();
        e.0 += n;
        if dir.len() == prefix.len() + child.len() {
            e.1 += n;
        }
    }
    let child_folders: Vec<Value> = children
        .iter()
        .map(|(p, (total, direct))| {
            json!({"folder_id": stable_id("FLD", p), "path": p, "name": p.rsplit(sep).next(), "uri": file_uri(p),
                   "files_recursive": total, "files_direct": direct})
        })
        .collect();
    let mut by_ext: BTreeMap<String, i64> = BTreeMap::new();
    let mut by_status: BTreeMap<String, i64> = BTreeMap::new();
    let mut bytes = 0i64;
    for f in &files {
        *by_ext
            .entry(f["ext"].as_str().unwrap_or("").to_string())
            .or_default() += 1;
        *by_status
            .entry(f["status"].as_str().unwrap_or("").to_string())
            .or_default() += 1;
        bytes += f["size"].as_i64().unwrap_or(0);
    }
    let mut q = c.prepare(
        "SELECT f.sha256, group_concat(g.path, char(10)) FROM files f JOIN files g ON g.sha256=f.sha256 AND g.status<>'missing'
         WHERE f.dir=?1 AND f.status<>'missing' AND f.sha256 IS NOT NULL AND f.size>0
         GROUP BY f.id HAVING COUNT(g.id)>1",
    )?;
    let mut dups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in q.query_map([&folder], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (sha, paths) = row?;
        let mut v: Vec<String> = paths.split('\n').map(String::from).collect();
        v.sort();
        dups.insert(sha, v);
    }
    let independent: std::collections::HashSet<String> = files
        .iter()
        .map(|f| match (&f["copy_of"], &f["derived_from"]["file_id"]) {
            (_, Value::Number(n)) => format!("d{n}"),
            (Value::Object(o), _) => format!("f{}", o["file_id"]),
            _ => format!("f{}", f["file_id"]),
        })
        .collect();
    Ok(json!({
        "schema": FOLDER_SCHEMA,
        "generator": {"name": "tpsearch", "version": env!("CARGO_PKG_VERSION")},
        "generated_at": db::iso(db::now()),
        "collection": collection,
        "folder": {
            "folder_id": stable_id("FLD", &folder), "path": folder, "name": folder.rsplit(sep).next(), "uri": file_uri(&folder),
            "root": root, "root_status": root_status,
            "direct_files": files.len(), "child_folders": child_folders.len(), "bytes": bytes,
            "missing_files": missing, "independent_sources": independent.len(),
            "counts_by_ext": by_ext, "counts_by_status": by_status,
        },
        "scan": last_run(c)?,
        "files": files,
        "child_folders": child_folders,
        "duplicate_groups": dups.into_iter().map(|(sha, paths)| json!({"sha256": sha, "paths": paths})).collect::<Vec<_>>(),
    }))
}

/// One report per folder that contains indexed files (and their ancestors up to
/// the root), written to `out_dir` with a manifest. Nothing is written in roots.
pub fn folders_recursive(
    collection: &str,
    index_dir: &Path,
    top: &str,
    out_dir: &Path,
) -> Result<usize> {
    let c = db::open(index_dir)?;
    let top = top.trim_end_matches(['/', '\\']).to_string();
    let sep = if top.contains('\\') { '\\' } else { '/' };
    let mut q = c.prepare("SELECT DISTINCT dir FROM files WHERE (dir=?1 OR substr(dir,1,length(?1)+1)=?1||?2) AND status<>'missing'")?;
    let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for d in q.query_map(params![top, sep.to_string()], |r| r.get::<_, String>(0))? {
        let mut d = d?;
        loop {
            dirs.insert(d.clone());
            if d.len() <= top.len() {
                break;
            }
            match d.rfind(sep) {
                Some(i) if i >= top.len() => d.truncate(i),
                _ => break,
            }
        }
    }
    dirs.insert(top.clone());
    fs::create_dir_all(out_dir)?;
    let mut manifest = vec![];
    for d in &dirs {
        let report = folder_with(&c, collection, d)?;
        let id = stable_id("FLD", d);
        let name = format!("{id}.json");
        fs::write(out_dir.join(&name), serde_json::to_vec_pretty(&report)?)?;
        manifest.push(json!({"folder_id": id, "path": d, "report": name}));
    }
    fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_vec_pretty(
            &json!({"schema": "tpsearch.folder_report_manifest/v1", "collection": collection, "top": top, "folders": manifest}),
        )?,
    )?;
    Ok(dirs.len())
}

pub fn resolve_folder(p: &str) -> String {
    dunce::canonicalize(PathBuf::from(p))
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_else(|_| p.to_string())
}
