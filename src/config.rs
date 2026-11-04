//! Collection settings. Stored as TOML outside every indexed root.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub collections: BTreeMap<String, Collection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Manual,
    Scheduled,
    Watched,
}

/// Independently selectable analysis capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Depth {
    /// File names, paths, sizes, dates, content hash. Always on.
    pub names: bool,
    /// Extracted document text, split into passages.
    pub text: bool,
    /// Existing structured metadata (frontmatter, JSON/YAML fields, DOCX properties).
    pub metadata: bool,
    /// Passage-level semantic search with a local model. Not available in stage 1.
    pub semantic: bool,
    /// Local AI classification/extraction. Not available in stage 1.
    pub local_ai: bool,
    /// Paid API enrichment. Not available in stage 1; requires `ai.allow_external`.
    pub paid_api: bool,
}
impl Default for Depth {
    fn default() -> Self {
        Depth {
            names: true,
            text: true,
            metadata: true,
            semantic: false,
            local_ai: false,
            paid_api: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Ai {
    /// "none", or a provider name once AI stages exist.
    pub provider: String,
    pub model: Option<String>,
    /// Must be true before any content may leave this machine.
    pub allow_external: bool,
    pub max_spend_usd: f64,
    /// Which content may be analysed: "none", "selected", "collection".
    pub scope: String,
}
impl Default for Ai {
    fn default() -> Self {
        Ai {
            provider: "none".into(),
            model: None,
            allow_external: false,
            max_spend_usd: 0.0,
            scope: "none".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Root {
    pub path: PathBuf,
    /// Overrides the collection depth for this root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Depth>,
}

/// A local embedding service; independent of the reserved AI enrichment settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingSettings {
    pub model: String,
    pub api_url: String,
    pub dims: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Collection {
    pub roots: Vec<Root>,
    /// Globs matched case-insensitively against root-relative paths (`/` separated).
    pub exclude: Vec<String>,
    /// Lower-case extensions without dots. Empty = inventory every file.
    pub extensions: Vec<String>,
    /// Larger files are inventoried (name, size, hash) but not extracted.
    pub max_file_size_mb: u64,
    /// Larger files are inventoried without a content hash (so no duplicate detection).
    pub hash_max_size_mb: u64,
    pub include_hidden: bool,
    /// Follow symbolic links and (on Windows) directory junctions.
    pub follow_symlinks: bool,
    pub mode: Mode,
    /// Interval for `scheduled` and `watched` modes (used by `tpsearch watch`).
    pub interval_minutes: u64,
    /// Worker threads for hashing and extraction.
    pub threads: usize,
    /// Pause between files per worker, to limit disk/CPU pressure on NAS volumes.
    pub throttle_ms: u64,
    /// Where the index lives. Default: per-user data directory.
    pub index_dir: Option<PathBuf>,
    pub depth: Depth,
    pub ai: Ai,
    /// Optional metadata profiles to apply (e.g. "research", "transcripts"). Reserved.
    pub profiles: Vec<String>,
    /// Command template for opening a location, e.g. `code -g {path}:{line}`.
    pub open_with: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<EmbeddingSettings>,
}
impl Default for Collection {
    fn default() -> Self {
        Collection {
            roots: vec![],
            exclude: default_excludes(),
            extensions: vec![],
            max_file_size_mb: 50,
            hash_max_size_mb: 4096,
            include_hidden: false,
            follow_symlinks: false,
            mode: Mode::Manual,
            interval_minutes: 60,
            threads: 2,
            throttle_ms: 0,
            index_dir: None,
            depth: Depth::default(),
            ai: Ai::default(),
            profiles: vec![],
            open_with: None,
            embedding: None,
        }
    }
}
fn default_excludes() -> Vec<String> {
    [
        "**/.git/**",
        "**/node_modules/**",
        "**/.tpsearch/**",
        "**/$RECYCLE.BIN/**",
        "**/System Volume Information/**",
        "**/target/**",
        "**/~$*",
    ]
    .map(String::from)
    .to_vec()
}

impl Collection {
    pub fn depth_for(&self, root: &Root) -> Depth {
        root.depth.clone().unwrap_or_else(|| self.depth.clone())
    }
    /// Reject settings this build cannot honour, so nothing is silently ignored
    /// and no content is ever sent externally.
    pub fn validate(&self) -> Result<()> {
        let depths =
            std::iter::once(&self.depth).chain(self.roots.iter().filter_map(|r| r.depth.as_ref()));
        for d in depths {
            if d.semantic || d.local_ai || d.paid_api {
                bail!("semantic search and AI analysis are not available in this build (stage 1); set semantic/local_ai/paid_api = false");
            }
        }
        if self.ai.provider != "none" {
            bail!("ai.provider must be \"none\" in this build (stage 1)");
        }
        anyhow::ensure!(self.threads >= 1, "threads must be at least 1");
        Ok(())
    }
}

pub fn valid_name(name: &str) -> Result<&str> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "collection names may contain only letters, digits, '-' and '_'"
    );
    Ok(name)
}

pub fn config_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("TPSEARCH_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    Ok(dirs::config_dir()
        .context("no per-user config directory; pass --config")?
        .join("tpsearch")
        .join("config.toml"))
}

pub fn default_index_dir(name: &str) -> Result<PathBuf> {
    let base = match std::env::var_os("TPSEARCH_DATA") {
        Some(p) => PathBuf::from(p),
        None => dirs::data_local_dir()
            .context("no per-user data directory; set index_dir")?
            .join("tpsearch"),
    };
    Ok(base.join("collections").join(name))
}

pub fn index_dir(name: &str, c: &Collection) -> Result<PathBuf> {
    match &c.index_dir {
        Some(p) => Ok(p.clone()),
        None => default_index_dir(name),
    }
}

pub fn load(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let s = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))
}

pub fn save(path: &Path, c: &Config) -> Result<()> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, toml::to_string_pretty(c)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

impl Config {
    pub fn get(&self, name: &str) -> Result<&Collection> {
        self.collections.get(name).with_context(|| {
            format!("no collection named {name:?}; see `tpsearch collection list`")
        })
    }
    pub fn get_mut(&mut self, name: &str) -> Result<&mut Collection> {
        self.collections
            .get_mut(name)
            .with_context(|| format!("no collection named {name:?}"))
    }
}

/// Apply `key = value` from the CLI to a collection.
pub fn set_key(c: &mut Collection, key: &str, value: &str) -> Result<()> {
    let b = || -> Result<bool> {
        value
            .parse()
            .with_context(|| format!("{key} expects true or false"))
    };
    let n = || -> Result<u64> {
        value
            .parse()
            .with_context(|| format!("{key} expects a number"))
    };
    let list = || -> Vec<String> {
        value
            .split(',')
            .map(|s| s.trim().trim_start_matches('.').to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    };
    match key {
        "extensions" => c.extensions = list(),
        "max_file_size_mb" => c.max_file_size_mb = n()?,
        "hash_max_size_mb" => c.hash_max_size_mb = n()?,
        "include_hidden" => c.include_hidden = b()?,
        "follow_symlinks" => c.follow_symlinks = b()?,
        "mode" => {
            c.mode = match value {
                "manual" => Mode::Manual,
                "scheduled" => Mode::Scheduled,
                "watched" => Mode::Watched,
                _ => bail!("mode must be manual, scheduled, or watched"),
            }
        }
        "interval_minutes" => c.interval_minutes = n()?.max(1),
        "threads" => c.threads = n()?.max(1) as usize,
        "throttle_ms" => c.throttle_ms = n()?,
        "index_dir" => c.index_dir = Some(PathBuf::from(value)),
        "open_with" => c.open_with = Some(value.into()),
        "depth.text" => c.depth.text = b()?,
        "depth.metadata" => c.depth.metadata = b()?,
        "depth.semantic" => c.depth.semantic = b()?,
        "depth.local_ai" => c.depth.local_ai = b()?,
        "depth.paid_api" => c.depth.paid_api = b()?,
        "profiles" => c.profiles = list(),
        _ => bail!("unknown setting {key:?}; edit the config file for nested settings"),
    }
    c.validate()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_roundtrip_and_reject_ai() {
        let mut cfg = Config::default();
        cfg.collections.insert("a".into(), Collection::default());
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.collections["a"], Collection::default());
        let mut c = Collection::default();
        assert!(set_key(&mut c, "depth.semantic", "true").is_err());
        let mut c = Collection::default();
        c.ai.provider = "openai".into();
        assert!(c.validate().is_err());
    }
}
