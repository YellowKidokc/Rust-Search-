//! SQLite store for a collection: files, passages (FTS5), metadata, roots, runs.
//! Lives in the collection's index_dir, never inside an indexed root.
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{fs, path::Path};

pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS roots(
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL DEFAULT 'unknown',      -- online | offline
  last_checked INTEGER,
  last_online INTEGER
);
CREATE TABLE IF NOT EXISTS files(
  id INTEGER PRIMARY KEY,
  root_id INTEGER NOT NULL,
  path TEXT NOT NULL UNIQUE,                    -- canonical, no \\?\ prefix
  dir TEXT NOT NULL,
  rel_path TEXT NOT NULL,                       -- relative to root, '/' separated
  name TEXT NOT NULL,
  ext TEXT NOT NULL,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,                       -- unix seconds
  mtime_ns INTEGER NOT NULL DEFAULT 0,
  sha256 TEXT,
  status TEXT NOT NULL,                         -- indexed | metadata_only | pending_adapter | too_large | failed | missing
  status_reason TEXT,
  extractor TEXT,
  extractor_version INTEGER,
  extracted_at INTEGER,
  passage_count INTEGER NOT NULL DEFAULT 0,
  representation TEXT NOT NULL DEFAULT 'original', -- original | copy | transcript | derived
  copy_of INTEGER,                              -- files.id of the first-seen identical file
  derived_from TEXT,                            -- declared source of a derived record
  derived_from_id INTEGER,
  first_seen INTEGER NOT NULL,
  last_seen_run INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS files_sha ON files(sha256);
CREATE INDEX IF NOT EXISTS files_dir ON files(dir);
CREATE INDEX IF NOT EXISTS files_root ON files(root_id);
CREATE TABLE IF NOT EXISTS passages(
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL,
  ord INTEGER NOT NULL,                         -- -1 = metadata passage
  loc_kind TEXT NOT NULL,                       -- line | paragraph
  loc_start INTEGER NOT NULL,
  loc_end INTEGER NOT NULL,
  section TEXT,
  t_start TEXT,
  t_end TEXT,
  page INTEGER
);
CREATE INDEX IF NOT EXISTS passages_file ON passages(file_id, ord);
CREATE VIRTUAL TABLE IF NOT EXISTS passages_fts USING fts5(text, section, tokenize='unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE IF NOT EXISTS names_fts USING fts5(name, rel_path, tokenize='unicode61 remove_diacritics 2');
CREATE TABLE IF NOT EXISTS meta(
  file_id INTEGER NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  origin TEXT NOT NULL DEFAULT 'source'         -- source = read from the file itself
);
CREATE INDEX IF NOT EXISTS meta_kv ON meta(key, value);
CREATE INDEX IF NOT EXISTS meta_file ON meta(file_id);
CREATE TABLE IF NOT EXISTS runs(
  id INTEGER PRIMARY KEY,
  started INTEGER NOT NULL,
  finished INTEGER,
  status TEXT NOT NULL,                         -- running | complete | interrupted
  stats TEXT
);
"#;

pub fn open(dir: &Path) -> Result<Connection> {
    fs::create_dir_all(dir).with_context(|| format!("creating index dir {}", dir.display()))?;
    let c = Connection::open(dir.join("index.sqlite"))?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    c.execute_batch(SCHEMA)?;
    let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
    anyhow::ensure!(
        v == 0 || v == SCHEMA_VERSION,
        "index schema v{v} is newer than this build (v{SCHEMA_VERSION})"
    );
    c.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(c)
}

pub fn exists(dir: &Path) -> bool {
    dir.join("index.sqlite").exists()
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Unix seconds → `YYYY-MM-DDTHH:MM:SSZ`.
pub fn iso(ts: i64) -> String {
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400);
    let (y, m, d) = civil(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
/// `YYYY-MM-DD` → unix seconds at 00:00 UTC.
pub fn parse_date(s: &str) -> Result<i64> {
    let p: Vec<i64> = s
        .split('-')
        .map(|x| x.parse())
        .collect::<Result<_, _>>()
        .context("dates must be YYYY-MM-DD")?;
    anyhow::ensure!(
        p.len() == 3 && (1..=12).contains(&p[1]) && (1..=31).contains(&p[2]),
        "dates must be YYYY-MM-DD"
    );
    let (y, m, d) = (p[0] - i64::from(p[1] <= 2), p[1], p[2]);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok((era * 146097 + doe - 719468) * 86400)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        let t = parse_date("2026-07-22").unwrap();
        assert_eq!(iso(t), "2026-07-22T00:00:00Z");
        assert_eq!(
            iso(parse_date("2000-02-29").unwrap()),
            "2000-02-29T00:00:00Z"
        );
    }
}
