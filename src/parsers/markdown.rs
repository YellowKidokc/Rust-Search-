use crate::model::{Definition, FileType, IndexRecord, TagWeight, TruthPredicate};
use anyhow::{Context, Result};
use serde_yaml::{Mapping, Value};
use std::{fs, path::Path, time::UNIX_EPOCH};

pub fn parse(path: &Path) -> Result<IndexRecord> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let (frontmatter, body) = split_frontmatter(&text);
    let yaml: Value = if frontmatter.is_empty() {
        Value::Mapping(Mapping::new())
    } else {
        serde_yaml::from_str(frontmatter)
            .with_context(|| format!("invalid YAML in {}", path.display()))?
    };
    let modified = fs::metadata(path)?
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (truth_predicates, definitions) = parse_tables(body);
    Ok(IndexRecord {
        file_path: path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned(),
        file_modified: modified,
        file_type: FileType::Markdown,
        frontmatter: yaml.clone(),
        title: string(&yaml, "title"),
        clean_title: string(&yaml, "clean_title"),
        paper_id: string(&yaml, "paper_id"),
        paper_uuid: string(&yaml, "paper_uuid"),
        series: string(&yaml, "series"),
        domain_primary: string(&yaml, "domain_primary"),
        domain_secondary: string(&yaml, "domain_secondary"),
        domain_tertiary: string(&yaml, "domain_tertiary"),
        content_type: string(&yaml, "content_type"),
        tags: strings(&yaml, "tags"),
        tags_weighted: weighted(&yaml),
        topic_keys: strings(&yaml, "topic_keys"),
        claim_ids: strings(&yaml, "claim_ids"),
        governing_question: string(&yaml, "governing_question"),
        one_sentence_finding: string(&yaml, "one_sentence_finding"),
        score_total: integer(&yaml, "score_total"),
        score_class: string(&yaml, "score_class"),
        grade: string(&yaml, "grade"),
        s01_net: integer(&yaml, "s01_net"),
        s02_net: integer(&yaml, "s02_net"),
        s03_net: integer(&yaml, "s03_net"),
        s04_net: integer(&yaml, "s04_net"),
        s05_net: integer(&yaml, "s05_net"),
        s06_net: integer(&yaml, "s06_net"),
        s07_net: integer(&yaml, "s07_net"),
        s08_net: integer(&yaml, "s08_net"),
        s09_net: integer(&yaml, "s09_net"),
        s10_net: integer(&yaml, "s10_net"),
        evd_support: integer(&yaml, "evd_support"),
        evd_counter: integer(&yaml, "evd_counter"),
        evd_balance: integer(&yaml, "evd_balance"),
        evd_weakest_claim: string(&yaml, "evd_weakest_claim"),
        coherence: float(&yaml, "coherence"),
        build_next: string(&yaml, "build_next"),
        truth_predicates,
        definitions,
        body_text: body.to_owned(),
    })
}

fn split_frontmatter(text: &str) -> (&str, &str) {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return ("", text);
    };
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return ("", text);
    }
    let start = first.len();
    let mut offset = start;
    for line in lines {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return (&text[start..offset], &text[offset + line.len()..]);
        }
        offset += line.len();
    }
    ("", text)
}

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_mapping()?.get(Value::String(key.into()))
}
fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
fn string(v: &Value, k: &str) -> Option<String> {
    get(v, k).and_then(scalar)
}
fn integer(v: &Value, k: &str) -> Option<i32> {
    get(v, k).and_then(|x| {
        x.as_i64()
            .map(|n| n as i32)
            .or_else(|| x.as_str()?.parse().ok())
    })
}
fn float(v: &Value, k: &str) -> Option<f64> {
    get(v, k).and_then(|x| x.as_f64().or_else(|| x.as_str()?.parse().ok()))
}
fn strings(v: &Value, k: &str) -> Vec<String> {
    get(v, k)
        .map(|x| match x {
            Value::Sequence(a) => a
                .iter()
                .filter_map(|x| {
                    scalar(x).or_else(|| {
                        x.as_mapping()
                            .and_then(|m| m.get(Value::String("tag".into())))
                            .and_then(scalar)
                    })
                })
                .collect(),
            _ => scalar(x).into_iter().collect(),
        })
        .unwrap_or_default()
}
fn weighted(v: &Value) -> Vec<TagWeight> {
    get(v, "tags_weighted")
        .and_then(Value::as_sequence)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let m = x.as_mapping()?;
                    Some(TagWeight {
                        tag: m.get(Value::String("tag".into())).and_then(scalar)?,
                        weight: m
                            .get(Value::String("weight".into()))
                            .and_then(Value::as_i64)
                            .unwrap_or_default() as i32,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|s| s.trim().to_owned())
        .collect()
}
fn separator(line: &str) -> bool {
    let c = cells(line);
    !c.is_empty()
        && c.iter()
            .all(|x| x.trim_matches([' ', ':', '-']).is_empty() && x.contains('-'))
}
fn parse_tables(body: &str) -> (Vec<TruthPredicate>, Vec<Definition>) {
    let lines: Vec<_> = body.lines().collect();
    let mut truths = vec![];
    let mut defs = vec![];
    let mut i = 0;
    while i < lines.len() {
        let h = cells(lines[i]);
        let truth = h.len() >= 6 && h[0] == "#" && h[1].eq_ignore_ascii_case("Truth Predicate");
        let def = h.len() >= 4
            && h[0].eq_ignore_ascii_case("Term")
            && h[1].eq_ignore_ascii_case("Plain definition");
        if truth || def {
            i += 1;
            if i < lines.len() && separator(lines[i]) {
                i += 1;
            }
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                let c = cells(lines[i]);
                if truth && c.len() >= 6 {
                    truths.push(TruthPredicate {
                        number: c[0].clone(),
                        predicate: c[1].clone(),
                        source_role: c[2].clone(),
                        modality: c[3].clone(),
                        formal_form: c[4].clone(),
                        warrant: c[5].clone(),
                    })
                } else if def && c.len() >= 4 {
                    defs.push(Definition {
                        term: c[0].clone(),
                        plain_definition: c[1].clone(),
                        domain: c[2].clone(),
                        first_used_in: c[3].clone(),
                    })
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    (truths, defs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frontmatter_and_tables() {
        let s="---\ntitle: Test\n---\n| # | Truth Predicate | Source Role | Modality | Formal form | Warrant |\n|---|---|---|---|---|---|\n| 1 | P | S | M | F | W |\n\n| Term | Plain definition | Domain | First used in |\n|---|---|---|---|\n| Logos | Word | Greek | 1 |\n";
        let (y, b) = split_frontmatter(s);
        assert!(y.contains("title"));
        let (t, d) = parse_tables(b);
        assert_eq!(t[0].predicate, "P");
        assert_eq!(d[0].term, "Logos");
    }
}
