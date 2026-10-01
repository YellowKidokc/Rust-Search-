use crate::model::Index;
use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

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
