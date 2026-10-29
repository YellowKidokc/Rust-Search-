//! Lexical (FTS5/BM25) passage search with filters, context, provenance and
//! quotation checks. Relevance is a ranking signal only.
use crate::{db, extract};
use anyhow::{bail, Result};
use rusqlite::{params_from_iter, types::Value, Connection, OptionalExtension};
use serde::Serialize;
use std::{collections::HashMap, fs, path::Path, time::UNIX_EPOCH};

pub const RELEVANCE_NOTE: &str =
    "lexical BM25 rank within one collection; not truth, evidence quality, or classification confidence";

#[derive(Debug, Clone, Default)]
pub struct Opts {
    pub query: String,
    pub phrase: bool,
    pub any: bool,
    pub ext: Vec<String>,
    pub path: Option<String>,
    pub under: Option<String>,
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub meta: Vec<(String, String)>,
    pub representation: Option<String>,
    pub limit: usize,
    pub include_missing: bool,
    pub verify: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Location {
    pub kind: String,
    pub start: i64,
    pub end: i64,
    pub section: Option<String>,
    pub t_start: Option<String>,
    pub t_end: Option<String>,
    pub page: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub collection: String,
    pub passage_id: i64,
    pub file_id: i64,
    pub path: String,
    pub rel_path: String,
    pub ext: String,
    pub modified: String,
    pub source_sha256: Option<String>,
    pub representation: String,
    pub file_status: String,
    pub root_offline: bool,
    /// Other indexed paths with byte-identical content (collapsed into this hit).
    pub copies: Vec<String>,
    pub location: Location,
    pub text: String,
    pub context_before: Option<String>,
    pub context_after: Option<String>,
    /// "text", "section heading", or "metadata".
    pub matched_in: String,
    pub matched_terms: Vec<String>,
    pub relevance: f64,
    pub quote_check: String,
    pub uri: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHit {
    pub collection: String,
    pub file_id: i64,
    pub path: String,
    pub status: String,
    pub representation: String,
    pub uri: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Results {
    pub query: String,
    pub fts_expression: String,
    pub relevance_note: &'static str,
    pub matching_passages: i64,
    pub matching_files: i64,
    /// Distinct source identities: byte-identical copies and declared
    /// derived records count once, with their original.
    pub independent_sources: i64,
    pub hits: Vec<Hit>,
    /// Files whose name or relative path matches (no passage needed).
    pub file_matches: Vec<FileHit>,
}

/// Turn user input into a safe FTS5 expression. `"quoted phrases"`, `-exclude`,
/// and `prefix*` are honoured; everything else is a literal term.
pub fn fts_query(input: &str, phrase: bool, any: bool) -> Result<(String, Vec<String>)> {
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut pos = vec![];
    let mut neg = vec![];
    let mut terms = vec![];
    if phrase {
        let t = input.trim().trim_matches('"');
        if t.is_empty() {
            bail!("empty query");
        }
        pos.push(quote(t));
        terms.push(t.to_string());
    } else {
        let mut rest = input.trim();
        while !rest.is_empty() {
            let negate = rest.starts_with('-') && rest.len() > 1;
            if negate {
                rest = &rest[1..];
            }
            let (item, is_phrase);
            if let Some(r) = rest.strip_prefix('"') {
                let end = r.find('"').unwrap_or(r.len());
                item = &r[..end];
                rest = r.get(end + 1..).unwrap_or("");
                is_phrase = true;
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                item = &rest[..end];
                rest = &rest[end..];
                is_phrase = false;
            }
            rest = rest.trim_start();
            let prefix = !is_phrase && item.len() > 1 && item.ends_with('*');
            let word = if prefix {
                &item[..item.len() - 1]
            } else {
                item
            };
            if word.trim().is_empty() {
                continue;
            }
            let expr = format!("{}{}", quote(word), if prefix { "*" } else { "" });
            if negate {
                neg.push(expr);
            } else {
                pos.push(expr);
                terms.push(word.to_string());
            }
        }
    }
    if pos.is_empty() {
        bail!("query needs at least one term that is not excluded");
    }
    let mut e = pos.join(if any { " OR " } else { " AND " });
    if !neg.is_empty() {
        e = format!("({e}) NOT {}", neg.join(" NOT "));
    }
    Ok((e, terms))
}

/// `file:` URI for a local or UNC path.
pub fn file_uri(path: &str) -> String {
    let p = path.replace('\\', "/");
    let enc: String = p
        .chars()
        .map(|c| match c {
            ' ' => "%20".into(),
            '#' => "%23".into(),
            '%' => "%25".into(),
            '?' => "%3F".into(),
            c => c.to_string(),
        })
        .collect();
    if let Some(unc) = enc.strip_prefix("//") {
        format!("file://{unc}")
    } else if enc.starts_with('/') {
        format!("file://{enc}")
    } else {
        format!("file:///{enc}")
    }
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn filters(o: &Opts, args: &mut Vec<Value>) -> String {
    let mut w = String::new();
    if !o.include_missing {
        w.push_str(" AND f.status<>'missing'");
    }
    if !o.ext.is_empty() {
        w.push_str(&format!(
            " AND f.ext IN ({})",
            vec!["?"; o.ext.len()].join(",")
        ));
        args.extend(
            o.ext
                .iter()
                .map(|e| Value::Text(e.trim_start_matches('.').to_lowercase())),
        );
    }
    if let Some(p) = &o.path {
        w.push_str(" AND f.path LIKE ? ESCAPE '\\'");
        args.push(Value::Text(format!("%{}%", like_escape(p))));
    }
    if let Some(p) = &o.under {
        w.push_str(" AND f.path LIKE ? ESCAPE '\\'");
        args.push(Value::Text(format!("{}%", like_escape(p))));
    }
    if let Some(t) = o.after {
        w.push_str(" AND f.mtime>=?");
        args.push(Value::Integer(t));
    }
    if let Some(t) = o.before {
        w.push_str(" AND f.mtime<?");
        args.push(Value::Integer(t));
    }
    if let Some(r) = &o.representation {
        w.push_str(" AND f.representation=?");
        args.push(Value::Text(r.clone()));
    }
    for (k, v) in &o.meta {
        w.push_str(" AND EXISTS(SELECT 1 FROM meta m WHERE m.file_id=f.id AND m.key=? AND m.value LIKE ? ESCAPE '\\')");
        args.push(Value::Text(k.clone()));
        args.push(Value::Text(format!("%{}%", like_escape(v))));
    }
    w
}

fn passage_text(c: &Connection, id: i64) -> Result<Option<String>> {
    Ok(
        c.query_row("SELECT text FROM passages_fts WHERE rowid=?1", [id], |r| {
            r.get(0)
        })
        .optional()?,
    )
}

fn neighbour(c: &Connection, file: i64, ord: i64) -> Result<Option<String>> {
    if ord < 0 {
        return Ok(None);
    }
    let id: Option<i64> = c
        .query_row(
            "SELECT id FROM passages WHERE file_id=?1 AND ord=?2",
            [file, ord],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match id {
        Some(id) => passage_text(c, id)?.map(|t| {
            let t: String = t.chars().take(240).collect();
            t.replace('\n', " ")
        }),
        None => None,
    })
}

/// Re-read the source and check the displayed text is really there.
fn quote_check(h: &Hit, extractor: Option<&str>, size: i64, mtime: i64) -> String {
    if h.root_offline {
        return "not checked: root offline".into();
    }
    if h.location.kind == "metadata" {
        return "not applicable: metadata fields".into();
    }
    let Ok(m) = fs::metadata(&h.path) else {
        return "source unavailable".into();
    };
    let mt = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    if m.len() as i64 != size || mt != Some(mtime) {
        return "source changed since indexing; re-run index".into();
    }
    let Some(a) = extractor.and_then(extract::adapter_by_name) else {
        return "not checked".into();
    };
    let Ok(bytes) = fs::read(&h.path) else {
        return "source unreadable".into();
    };
    if a.verifiable_by_lines {
        let text = extract::decode(&bytes);
        let lines: Vec<&str> = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        let (s, e) = (
            h.location.start.max(1) as usize - 1,
            (h.location.end as usize).min(lines.len()),
        );
        let window = lines.get(s..e).unwrap_or_default();
        let mut it = window.iter();
        let ok = h.text.split('\n').all(|want| it.any(|have| *have == want));
        if ok {
            "verified against source lines".into()
        } else {
            "MISMATCH with source lines".into()
        }
    } else {
        use sha2::{Digest, Sha256};
        let now = format!("{:x}", Sha256::digest(&bytes));
        if h.source_sha256.as_deref() == Some(now.as_str()) {
            format!("source hash matches extraction ({} adapter)", a.name)
        } else {
            "source changed since indexing; re-run index".into()
        }
    }
}

pub fn run(collection: &str, index_dir: &Path, o: &Opts) -> Result<Results> {
    anyhow::ensure!(
        db::exists(index_dir),
        "collection {collection:?} has not been indexed; run `tpsearch index {collection}`"
    );
    let c = db::open(index_dir)?;
    let (expr, terms) = fts_query(&o.query, o.phrase, o.any)?;
    let mut args = vec![Value::Text(expr.clone())];
    let w = filters(o, &mut args);
    let from = format!(
        "FROM passages_fts JOIN passages p ON p.id=passages_fts.rowid JOIN files f ON f.id=p.file_id
         JOIN roots r ON r.id=f.root_id LEFT JOIN files df ON df.id=f.derived_from_id
         WHERE passages_fts MATCH ?{w}"
    );
    let mut res = Results {
        query: o.query.clone(),
        fts_expression: expr.clone(),
        relevance_note: RELEVANCE_NOTE,
        ..Default::default()
    };
    (res.matching_passages, res.matching_files, res.independent_sources) = c.query_row(
        &format!(
            "SELECT COUNT(*), COUNT(DISTINCT f.id), COUNT(DISTINCT CASE WHEN f.derived_from_id IS NOT NULL
               THEN COALESCE(df.copy_of, df.id) ELSE COALESCE(f.copy_of, f.id) END) {from}"
        ),
        params_from_iter(args.iter()),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut q = c.prepare(&format!(
        "SELECT p.id, p.file_id, p.ord, p.loc_kind, p.loc_start, p.loc_end, p.section, p.t_start, p.t_end, p.page,
                passages_fts.text, bm25(passages_fts, 1.0, 0.5), f.path, f.rel_path, f.ext, f.sha256, f.representation,
                f.status, f.mtime, f.extractor, r.status, f.size,
                highlight(passages_fts, 1, char(1), char(2)) {from}
         ORDER BY bm25(passages_fts, 1.0, 0.5), p.id LIMIT ?"
    ))?;
    let mut a2 = args.clone();
    a2.push(Value::Integer((o.limit.max(1) * 5) as i64));
    let rows = q.query_map(params_from_iter(a2.iter()), |r| {
        let sec_hl: Option<String> = r.get(22)?;
        let ord: i64 = r.get(2)?;
        let kind: String = r.get(3)?;
        Ok((
            Hit {
                collection: collection.into(),
                passage_id: r.get(0)?,
                file_id: r.get(1)?,
                location: Location {
                    kind: kind.clone(),
                    start: r.get(4)?,
                    end: r.get(5)?,
                    section: r.get(6)?,
                    t_start: r.get(7)?,
                    t_end: r.get(8)?,
                    page: r.get(9)?,
                },
                text: r.get(10)?,
                relevance: -r.get::<_, f64>(11)?,
                path: r.get(12)?,
                rel_path: r.get(13)?,
                ext: r.get(14)?,
                source_sha256: r.get(15)?,
                representation: r.get(16)?,
                file_status: r.get(17)?,
                modified: db::iso(r.get(18)?),
                root_offline: r.get::<_, String>(20)? == "offline",
                matched_in: if kind == "metadata" {
                    "metadata".into()
                } else if sec_hl.is_some_and(|s| s.contains('\u{1}')) {
                    "section heading".into()
                } else {
                    "text".into()
                },
                copies: vec![],
                context_before: None,
                context_after: None,
                matched_terms: vec![],
                quote_check: String::new(),
                uri: String::new(),
            },
            ord,
            r.get::<_, Option<String>>(19)?,
            r.get::<_, i64>(21)?,
            r.get::<_, i64>(18)?,
        ))
    })?;
    // Collapse byte-identical copies: same content hash + same passage ordinal.
    let mut seen: HashMap<(String, i64), usize> = HashMap::new();
    let mut extra = vec![];
    for row in rows {
        let (mut h, ord, extractor, size, mtime) = row?;
        if let Some(sha) = &h.source_sha256 {
            if let Some(&i) = seen.get(&(sha.clone(), ord)) {
                if i < res.hits.len() {
                    res.hits[i].copies.push(h.path.clone());
                }
                continue;
            }
            seen.insert((sha.clone(), ord), res.hits.len().min(o.limit));
        }
        if res.hits.len() >= o.limit {
            continue;
        }
        let lower = h.text.to_lowercase();
        h.matched_terms = terms
            .iter()
            .filter(|t| lower.contains(&t.to_lowercase()))
            .cloned()
            .collect();
        h.context_before = neighbour(&c, h.file_id, ord - 1)?;
        h.context_after = if ord >= 0 {
            neighbour(&c, h.file_id, ord + 1)?
        } else {
            None
        };
        h.uri = file_uri(&h.path);
        extra.push((extractor, size, mtime));
        res.hits.push(h);
    }
    if o.verify {
        for (h, (ex, size, mtime)) in res.hits.iter_mut().zip(&extra) {
            h.quote_check = quote_check(h, ex.as_deref(), *size, *mtime);
        }
    }
    let mut args = vec![Value::Text(expr)];
    let w = filters(o, &mut args);
    args.push(Value::Integer(o.limit as i64));
    let mut q = c.prepare(&format!(
        "SELECT f.id, f.path, f.status, f.representation FROM names_fts JOIN files f ON f.id=names_fts.rowid
         JOIN roots r ON r.id=f.root_id LEFT JOIN files df ON df.id=f.derived_from_id
         WHERE names_fts MATCH ?{w} ORDER BY bm25(names_fts), f.id LIMIT ?"
    ))?;
    res.file_matches = q
        .query_map(params_from_iter(args.iter()), |r| {
            let path: String = r.get(1)?;
            Ok(FileHit {
                collection: collection.into(),
                file_id: r.get(0)?,
                uri: file_uri(&path),
                path,
                status: r.get(2)?,
                representation: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_building() {
        assert_eq!(
            fts_query("grace field", false, false).unwrap().0,
            "\"grace\" AND \"field\""
        );
        assert_eq!(
            fts_query("\"free will\" -doubt ent*", false, false)
                .unwrap()
                .0,
            "(\"free will\" AND \"ent\"*) NOT \"doubt\""
        );
        assert_eq!(fts_query("a\"b", true, false).unwrap().0, "\"a\"\"b\"");
        assert!(fts_query("-x", false, false).is_err());
        assert_eq!(fts_query("a b", false, true).unwrap().0, "\"a\" OR \"b\"");
    }
    #[test]
    fn uris() {
        assert_eq!(
            file_uri(r"C:\My Docs\a#1.md"),
            "file:///C:/My%20Docs/a%231.md"
        );
        assert_eq!(file_uri(r"\\nas\share\x.md"), "file://nas/share/x.md");
        assert_eq!(file_uri("/home/u/x.md"), "file:///home/u/x.md");
    }
}
