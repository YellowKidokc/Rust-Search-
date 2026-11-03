use crate::model::{FileType, IndexRecord};
use anyhow::{Context, Result};
use std::{fs, path::Path};
pub fn parse(path: &Path) -> Result<IndexRecord> {
    let body = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value = serde_yaml::from_str(&body)?;
    super::structured(path, FileType::Yaml, value, body)
}
