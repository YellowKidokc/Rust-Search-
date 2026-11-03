use crate::model::{FileType, IndexRecord};
use anyhow::{Context, Result};
use scraper::{Html, Selector};
use std::{fs, path::Path};
pub fn parse(path: &Path) -> Result<IndexRecord> {
    let source = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let doc = Html::parse_document(&source);
    let selector = Selector::parse("body").expect("valid selector");
    let text = doc
        .select(&selector)
        .flat_map(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    super::bare(path, FileType::Html, text)
}
