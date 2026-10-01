use crate::model::Index;
use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(serde::Serialize)]
pub struct IndexInfo {
    pub name: String,
    pub roots: Vec<String>,
    pub record_count: usize,
    pub indexed_at: Option<String>,
}

pub fn home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("TPSEARCH_HOME") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".tpsearch"))
}
pub fn validate_name(name: &str) -> Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "index names may contain only letters, digits, '-' and '_'"
    );
    Ok(())
}
pub fn named_base(name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(home()?.join("indexes").join(name))
}
pub fn list_named() -> Result<Vec<IndexInfo>> {
    let dir = home()?.join("indexes");
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for entry in fs::read_dir(dir)?.filter_map(Result::ok) {
        if let Ok(index) = load(&entry.path()) {
            out.push(IndexInfo {
                name: index
                    .name
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned()),
                roots: index.roots,
                record_count: index.records.len(),
                indexed_at: index.indexed_at,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
pub fn load_named(name: &str) -> Result<Index> {
    if name != "all" {
        return load(&named_base(name)?);
    }
    let mut indexes = vec![];
    for info in list_named()? {
        indexes.push((info.name.clone(), load(&named_base(&info.name)?)?));
    }
    Ok(merge_named(indexes))
}
pub fn merge_named(indexes: Vec<(String, Index)>) -> Index {
    let mut merged = Index::default();
    let mut seen = HashSet::new();
    for (name, mut idx) in indexes {
        merged.roots.append(&mut idx.roots);
        for r in idx.records {
            if seen.insert(r.file_path.clone()) {
                merged
                    .record_indexes
                    .insert(r.file_path.clone(), name.clone());
                merged.records.push(r);
            }
        }
    }
    merged.name = Some("all".into());
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_merge_deduplicates_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        let record = crate::parsers::text::parse(&file).unwrap();
        let a = Index {
            records: vec![record.clone()],
            ..Index::default()
        };
        let b = Index {
            records: vec![record],
            ..Index::default()
        };
        let merged = merge_named(vec![("a".into(), a), ("b".into(), b)]);
        assert_eq!(merged.records.len(), 1);
        assert_eq!(merged.record_indexes.values().next().unwrap(), "a");
    }
}

pub fn base_for_root(root: &Path) -> PathBuf {
    if root.is_file() {
        root.parent().unwrap_or(Path::new("."))
    } else {
        root
    }
    .join(".tpsearch")
}
pub fn find_base(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(if p.file_name().is_some_and(|name| name == "index.json") {
            p.parent().unwrap_or(Path::new(".")).to_path_buf()
        } else if p.file_name().is_some_and(|name| name == ".tpsearch") {
            p.to_path_buf()
        } else if p.is_dir() {
            p.join(".tpsearch")
        } else {
            p.to_path_buf()
        });
    }
    let mut p = std::env::current_dir()?;
    loop {
        let c = p.join(".tpsearch/index.json");
        if c.exists() {
            return Ok(p.join(".tpsearch"));
        }
        if !p.pop() {
            break;
        }
    }
    anyhow::bail!("no index found; run `tpsearch index <PATH>`, or pass --index <INDEX_FILE>")
}
pub fn load(base: &Path) -> Result<Index> {
    let p = base.join("index.json");
    let data = fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
    serde_json::from_slice(&data).context("decoding index")
}
pub fn save(base: &Path, index: &Index) -> Result<()> {
    fs::create_dir_all(base)?;
    let p = base.join("index.json");
    let tmp = base.join("index.json.tmp");
    fs::write(&tmp, serde_json::to_vec(index)?)?;
    fs::rename(tmp, p)?;
    Ok(())
}
