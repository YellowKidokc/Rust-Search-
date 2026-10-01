use crate::model::SearchResult;
use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
pub fn parse_selection(input: &str, max: usize) -> Result<Vec<usize>> {
    let mut set = HashSet::new();
    for part in input.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((a, b)) = part.split_once('-') {
            let (a, b): (usize, usize) = (a.parse()?, b.parse()?);
            anyhow::ensure!(a >= 1 && a <= b && b <= max, "selection out of range");
            for n in a..=b {
                set.insert(n - 1);
            }
        } else {
            let n: usize = part.parse()?;
            anyhow::ensure!(n >= 1 && n <= max, "selection out of range");
            set.insert(n - 1);
        }
    }
    let mut v = set.into_iter().collect::<Vec<_>>();
    v.sort();
    Ok(v)
}
pub fn copy(
    results: &[SearchResult],
    target: &Path,
    flatten: bool,
    roots: &[String],
) -> Result<()> {
    fs::create_dir_all(target)?;
    for r in results {
        let src = Path::new(&r.record.file_path);
        let rel = if flatten {
            PathBuf::from(src.file_name().context("source has no filename")?)
        } else {
            roots
                .iter()
                .filter_map(|root| src.strip_prefix(root).ok())
                .min_by_key(|p| p.components().count())
                .unwrap_or_else(|| Path::new(src.file_name().unwrap_or_default()))
                .to_owned()
        };
        let dest = target.join(rel);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?
        }
        fs::copy(src, &dest)
            .with_context(|| format!("copying {} to {}", src.display(), dest.display()))?;
    }
    Ok(())
}
#[derive(Serialize)]
struct ManifestRow<'a> {
    path: &'a str,
    title: &'a str,
    score: Option<i32>,
    matched_fields: &'a [String],
}
pub fn manifest(results: &[SearchResult], path: &Path) -> Result<()> {
    let rows = results
        .iter()
        .map(|r| ManifestRow {
            path: &r.record.file_path,
            title: r
                .record
                .title
                .as_deref()
                .or(r.record.clean_title.as_deref())
                .unwrap_or("Untitled"),
            score: r.record.score_total,
            matched_fields: &r.matched_fields,
        })
        .collect::<Vec<_>>();
    fs::write(path, serde_json::to_vec_pretty(&rows)?)?;
    Ok(())
}
pub fn csv(results: &[SearchResult], path: &Path) -> Result<()> {
    let mut w = csv::Writer::from_path(path)?;
    w.write_record([
        "title",
        "score",
        "s01",
        "s02",
        "s03",
        "s04",
        "s05",
        "s06",
        "s07",
        "s08",
        "s09",
        "s10",
        "domain",
        "series",
        "file_path",
        "one_sentence_finding",
        "evd_balance",
        "tags",
    ])?;
    for x in results {
        let r = &x.record;
        let n = |x: Option<i32>| x.map(|x| x.to_string()).unwrap_or_default();
        w.write_record([
            r.title
                .as_deref()
                .or(r.clean_title.as_deref())
                .unwrap_or(""),
            &n(r.score_total),
            &n(r.s01_net),
            &n(r.s02_net),
            &n(r.s03_net),
            &n(r.s04_net),
            &n(r.s05_net),
            &n(r.s06_net),
            &n(r.s07_net),
            &n(r.s08_net),
            &n(r.s09_net),
            &n(r.s10_net),
            r.domain_primary.as_deref().unwrap_or(""),
            r.series.as_deref().unwrap_or(""),
            &r.file_path,
            r.one_sentence_finding.as_deref().unwrap_or(""),
            &n(r.evd_balance),
            &r.tags.join(";"),
        ])?;
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selections() {
        assert_eq!(parse_selection("1,3,5-7", 7).unwrap(), vec![0, 2, 4, 5, 6]);
        assert!(parse_selection("0", 2).is_err());
    }
}
