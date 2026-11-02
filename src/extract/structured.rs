use super::{decode, flatten_meta, text::split_lines, Extracted};
use anyhow::{Context, Result};
use std::path::Path;

/// Keys that mark a JSON/YAML file as derived analysis of another source.
const SOURCE_KEYS: [&str; 6] = [
    "source_path",
    "source_file",
    "original_path",
    "source",
    "source_sha256",
    "sha256",
];

fn build(text: &str, v: serde_yaml::Value) -> Extracted {
    let mut meta = vec![];
    flatten_meta(&v, "", &mut meta, 0);
    let derived_from = SOURCE_KEYS.iter().find_map(|k| {
        meta.iter()
            .find(|(mk, val)| mk == k && !val.is_empty() && !val.starts_with("http"))
            .map(|(_, v)| v.clone())
    });
    Extracted {
        passages: split_lines(text, 1, false),
        meta,
        derived_from,
    }
}

pub(super) fn json(bytes: &[u8], _: &Path) -> Result<Extracted> {
    let text = decode(bytes);
    let v: serde_json::Value = serde_json::from_str(&text).context("invalid JSON")?;
    Ok(build(&text, serde_yaml::to_value(v)?))
}

pub(super) fn yaml(bytes: &[u8], _: &Path) -> Result<Extracted> {
    let text = decode(bytes);
    let v: serde_yaml::Value = serde_yaml::from_str(&text).context("invalid YAML")?;
    Ok(build(&text, v))
}
