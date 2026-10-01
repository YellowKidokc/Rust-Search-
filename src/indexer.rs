use crate::{
    model::{Index, IndexRecord},
    parsers, store,
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
pub fn run(roots: &[PathBuf], name: Option<&str>) -> Result<(PathBuf, Stats)> {
    anyhow::ensure!(!roots.is_empty(), "at least one path is required");
    for p in roots {
        anyhow::ensure!(p.exists(), "path does not exist: {}", p.display());
    }
    let base = match name {
        Some(n) => store::named_base(n)?,
        None => store::base_for_root(&roots[0]),
    };
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
        for path in supported_paths(root) {
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
            match parse_path(&path) {
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
    store::save(
        &base,
        &Index {
            roots,
            records,
            name: name.map(str::to_owned),
            indexed_at: Some(chrono::Utc::now().to_rfc3339()),
            record_indexes: HashMap::new(),
        },
    )?;
    Ok((base, st))
}
pub fn is_supported(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|x| {
        matches!(
            x.to_ascii_lowercase().as_str(),
            "md" | "txt" | "json" | "yaml" | "yml" | "pdf" | "docx" | "html" | "htm"
        )
    })
}
pub fn parse_path(path: &Path) -> Result<IndexRecord> {
    match path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "md" => parsers::markdown::parse(path),
        "txt" => parsers::text::parse(path),
        "json" => parsers::json::parse(path),
        "yaml" | "yml" => parsers::yaml::parse(path),
        "pdf" => parsers::pdf::parse(path),
        "docx" => parsers::docx::parse(path),
        "html" | "htm" => parsers::html::parse(path),
        _ => anyhow::bail!("unsupported file: {}", path.display()),
    }
}
fn supported_paths(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return is_supported(root)
            .then(|| root.to_owned())
            .into_iter()
            .collect();
    }
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".tpsearch")
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && is_supported(e.path()))
        .map(|e| e.into_path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexes_markdown_text_and_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "---\ntitle: Markdown\n---\nalpha").unwrap();
        std::fs::write(dir.path().join("b.txt"), "plain beta").unwrap();
        std::fs::write(
            dir.path().join("c.json"),
            r#"{"title":"JSON title","score_total":72,"note":"gamma"}"#,
        )
        .unwrap();
        let (_, stats) = run(&[dir.path().to_owned()], None).unwrap();
        let index = store::load(&dir.path().join(".tpsearch")).unwrap();
        assert_eq!(stats.added, 3);
        assert_eq!(index.records.len(), 3);
        assert!(index
            .records
            .iter()
            .any(|r| r.title.as_deref() == Some("JSON title") && r.score_total == Some(72)));
        let q = crate::query::Query {
            search: Some("beta".into()),
            limit: 10,
            ..Default::default()
        };
        assert_eq!(crate::query::execute(&index, &q).len(), 1);
    }
}
