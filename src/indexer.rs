use crate::{
    model::{Index, IndexRecord},
    parsers::markdown,
    store,
};
use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

pub struct Stats {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub failed: usize,
}
pub fn run(roots: &[PathBuf]) -> Result<(PathBuf, Stats)> {
    anyhow::ensure!(!roots.is_empty(), "at least one path is required");
    for p in roots {
        anyhow::ensure!(p.exists(), "path does not exist: {}", p.display());
    }
    let base = store::base_for_root(&roots[0]);
    let old = store::load(&base).unwrap_or_default();
    let mut prior: HashMap<String, IndexRecord> = old
        .records
        .into_iter()
        .map(|r| (r.file_path.clone(), r))
        .collect();
    let mut records = vec![];
    let mut seen = HashSet::new();
    let mut st = Stats {
        added: 0,
        updated: 0,
        unchanged: 0,
        removed: 0,
        failed: 0,
    };
    for root in roots {
        for path in markdown_paths(root) {
            let canonical = path.canonicalize().unwrap_or(path.clone());
            let key = canonical.to_string_lossy().into_owned();
            if !seen.insert(key.clone()) {
                continue;
            }
            let mtime = std::fs::metadata(&path)?
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if prior.get(&key).is_some_and(|r| r.file_modified == mtime) {
                records.push(prior.remove(&key).unwrap());
                st.unchanged += 1;
                continue;
            }
            match markdown::parse(&path) {
                Ok(r) => {
                    if prior.remove(&key).is_some() {
                        st.updated += 1
                    } else {
                        st.added += 1
                    }
                    records.push(r)
                }
                Err(e) => {
                    eprintln!("warning: {e:#}");
                    st.failed += 1;
                    if let Some(r) = prior.remove(&key) {
                        records.push(r)
                    }
                }
            }
        }
    }
    st.removed = prior.len();
    records.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    let roots = roots
        .iter()
        .map(|p| {
            p.canonicalize()
                .unwrap_or_else(|_| p.clone())
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    store::save(&base, &Index { roots, records })?;
    Ok((base, st))
}
fn markdown_paths(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return (root
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("md")))
        .then(|| root.to_owned())
        .into_iter()
        .collect();
    }
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".tpsearch")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("md"))
        })
        .map(|e| e.into_path())
        .collect()
}
