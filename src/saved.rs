use crate::{model::SearchResult, query::Query};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
#[derive(Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    pub query: Query,
    pub results: Vec<SearchResult>,
}
fn safe(name: &str) -> Result<&str> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "saved search names may contain only letters, digits, '-' and '_'"
    );
    Ok(name)
}
pub fn save(base: &Path, name: &str, query: &Query, results: &[SearchResult]) -> Result<()> {
    let name = safe(name)?;
    let dir = base.join("saved");
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&SavedSearch {
            name: name.into(),
            query: query.clone(),
            results: results.to_vec(),
        })?,
    )?;
    Ok(())
}
pub fn load(base: &Path, name: &str) -> Result<SavedSearch> {
    let name = safe(name)?;
    let p = base.join("saved").join(format!("{name}.json"));
    serde_json::from_slice(&fs::read(&p).with_context(|| format!("reading {}", p.display()))?)
        .context("decoding saved search")
}
pub fn list(base: &Path) -> Result<Vec<String>> {
    let dir = base.join("saved");
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.path().file_stem()?.to_str().map(str::to_owned))
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}
