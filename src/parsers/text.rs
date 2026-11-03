use crate::model::{FileType, IndexRecord};
use anyhow::{Context, Result};
use std::{fs, path::Path};
pub fn parse(path: &Path) -> Result<IndexRecord> {
    super::bare(
        path,
        FileType::Text,
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
}
