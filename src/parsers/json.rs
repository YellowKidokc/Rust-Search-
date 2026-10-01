use crate::model::{FileType, IndexRecord};
use anyhow::{Context, Result};
use std::{fs, path::Path};
pub fn parse(path: &Path) -> Result<IndexRecord> {
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let body = serde_json::to_string_pretty(&value)?;
    super::structured(path, FileType::Json, serde_yaml::to_value(value)?, body)
}
