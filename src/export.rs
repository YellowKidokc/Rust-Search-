use crate::{
    model::{IndexRecord, SearchResult},
    query::Query,
};
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

pub fn snippets(results: &[SearchResult], query: &Query, path: &Path) -> Result<()> {
    fs::write(path, snippets_markdown(results, query)?)?;
    Ok(())
}

pub fn snippets_markdown(results: &[SearchResult], query: &Query) -> Result<String> {
    let description = query.search.as_deref().unwrap_or("filtered results");
    let serialized = serde_json::to_string(query)?;
    let mut out = format!(
        "# Search Results: {description}\nGenerated: {}\nQuery: `{serialized}`\nResults: {}\n",
        chrono::Utc::now().to_rfc3339(),
        results.len()
    );
    for (i, result) in results.iter().enumerate() {
        let r = &result.record;
        let title = r
            .title
            .as_deref()
            .or(r.clean_title.as_deref())
            .unwrap_or("Untitled");
        out.push_str(&format!(
            "\n---\n\n## {}. {} — Score: {}\n**File:** `{}`\n**Matched in:** {}\n\n{}\n",
            i + 1,
            title,
            r.score_total
                .map(|x| x.to_string())
                .unwrap_or_else(|| "-".into()),
            r.file_path,
            result.matched_fields.join(", "),
            extract_snippet(r, &result.matched_fields, query.search.as_deref())
        ));
    }
    Ok(out)
}

fn extract_snippet(r: &IndexRecord, fields: &[String], search: Option<&str>) -> String {
    let Some(term) = search else {
        let mut pieces = vec![];
        if let Some(x) = &r.one_sentence_finding {
            pieces.push(format!("**One-sentence finding:** {x}"));
        }
        if let Some(x) = r.truth_predicates.first() {
            pieces.push(truth_row(x));
        }
        return if pieces.is_empty() {
            first_chars(&r.body_text, 500)
        } else {
            pieces.join("\n\n")
        };
    };
    let has = |s: &str| s.to_lowercase().contains(&term.to_lowercase());
    let mut pieces = vec![];
    if fields.iter().any(|x| x == "truth_predicate") {
        pieces.extend(
            r.truth_predicates
                .iter()
                .filter(|x| {
                    has(&format!(
                        "{} {} {} {} {}",
                        x.predicate, x.source_role, x.modality, x.formal_form, x.warrant
                    ))
                })
                .map(truth_row),
        );
    }
    if fields.iter().any(|x| x == "definition") {
        pieces.extend(r.definitions.iter().filter(|x| has(&format!("{} {} {} {}",x.term,x.plain_definition,x.domain,x.first_used_in))).map(|x| format!("| Term | Plain definition | Domain | First used in |\n|---|---|---|---|\n| {} | {} | {} | {} |",x.term,x.plain_definition,x.domain,x.first_used_in)));
    }
    for (name, value) in [
        ("one_sentence_finding", &r.one_sentence_finding),
        ("governing_question", &r.governing_question),
    ] {
        if fields.iter().any(|x| x == name) {
            if let Some(v) = value {
                pieces.push(format!("**{}:** {}", name.replace('_', " "), v));
            }
        }
    }
    if fields.iter().any(|x| x == "frontmatter") {
        if let Some(map) = r.frontmatter.as_mapping() {
            for (k, v) in map {
                let rendered = serde_yaml::to_string(v).unwrap_or_default();
                if has(&rendered) {
                    pieces.push(format!(
                        "```yaml\n{}: {}\n```",
                        k.as_str().unwrap_or("field"),
                        rendered.trim()
                    ));
                }
            }
        }
    }
    if fields.iter().any(|x| x == "body") {
        let paragraphs: Vec<_> = r.body_text.split("\n\n").collect();
        if let Some(at) = paragraphs.iter().position(|p| has(p)) {
            pieces.push(
                paragraphs[at.saturating_sub(1)..=(at + 1).min(paragraphs.len() - 1)].join("\n\n"),
            );
        }
    }
    if pieces.is_empty() {
        first_chars(&r.body_text, 500)
    } else {
        pieces.join("\n\n")
    }
}
fn truth_row(x: &crate::model::TruthPredicate) -> String {
    format!("| # | Truth Predicate | Source Role | Modality | Formal form | Warrant |\n|---|---|---|---|---|---|\n| {} | {} | {} | {} | {} | {} |",x.number,x.predicate,x.source_role,x.modality,x.formal_form,x.warrant)
}
fn first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selections() {
        assert_eq!(parse_selection("1,3,5-7", 7).unwrap(), vec![0, 2, 4, 5, 6]);
        assert!(parse_selection("0", 2).is_err());
    }

    #[test]
    fn paragraph_context() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "before\n\nneedle here\n\nafter\n\nlast").unwrap();
        let record = crate::parsers::text::parse(&file).unwrap();
        let result = SearchResult {
            record,
            matched_fields: vec!["body".into()],
            snippet: String::new(),
            index_name: None,
        };
        let query = Query {
            search: Some("needle".into()),
            limit: 20,
            ..Query::default()
        };
        let output = snippets_markdown(&[result], &query).unwrap();
        assert!(output.contains("before\n\nneedle here\n\nafter"));
        assert!(!output.contains("\n\nlast\n"));
    }
}
