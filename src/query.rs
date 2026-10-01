use crate::model::{Index, IndexRecord, SearchResult};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Query {
    pub tag: Option<String>,
    pub topic: Option<String>,
    pub domain: Option<String>,
    pub series: Option<String>,
    pub claim: Option<String>,
    pub search: Option<String>,
    pub min_score: Option<i32>,
    pub min_sections: [Option<i32>; 10],
    pub min_evd_support: Option<i32>,
    pub has_counter: Option<bool>,
    pub sort: String,
    pub limit: usize,
}
fn has(v: &str, q: &str) -> bool {
    v.to_lowercase().contains(&q.to_lowercase())
}
fn opt(v: &Option<String>, q: &str) -> bool {
    v.as_deref().is_some_and(|v| has(v, q))
}
fn searchable(r: &IndexRecord) -> Vec<(&'static str, String)> {
    let mut v = vec![];
    for (n, x) in [
        ("one_sentence_finding", &r.one_sentence_finding),
        ("governing_question", &r.governing_question),
        ("evd_weakest_claim", &r.evd_weakest_claim),
    ] {
        if let Some(x) = x {
            v.push((n, x.clone()))
        }
    }
    for x in &r.truth_predicates {
        v.push((
            "truth_predicate",
            format!(
                "{} {} {} {} {}",
                x.predicate, x.source_role, x.modality, x.formal_form, x.warrant
            ),
        ))
    }
    for x in &r.definitions {
        v.push((
            "definition",
            format!(
                "{} {} {} {}",
                x.term, x.plain_definition, x.domain, x.first_used_in
            ),
        ))
    }
    v.push(("body", r.body_text.clone()));
    // Preserve flexible, non-schema claim text fields in search without forcing
    // every producer's frontmatter into a fixed Rust structure.
    if let Ok(frontmatter) = serde_yaml::to_string(&r.frontmatter) {
        v.push(("frontmatter", frontmatter));
    }
    v
}
pub fn execute(index: &Index, q: &Query) -> Vec<SearchResult> {
    let mut out: Vec<_> = index
        .records
        .iter()
        .filter_map(|r| {
            let mut fields = vec![];
            macro_rules! req {
                ($cond:expr,$name:expr) => {
                    if !$cond {
                        return None;
                    } else {
                        fields.push($name.into())
                    }
                };
            }
            if let Some(x) = &q.tag {
                req!(
                    r.tags.iter().any(|v| has(v, x))
                        || r.tags_weighted.iter().any(|v| has(&v.tag, x)),
                    "tags"
                )
            }
            if let Some(x) = &q.topic {
                req!(r.topic_keys.iter().any(|v| has(v, x)), "topic_keys")
            }
            if let Some(x) = &q.domain {
                req!(
                    [&r.domain_primary, &r.domain_secondary, &r.domain_tertiary]
                        .iter()
                        .any(|v| opt(v, x)),
                    "domain"
                )
            }
            if let Some(x) = &q.series {
                req!(opt(&r.series, x), "series")
            }
            if let Some(x) = &q.claim {
                req!(
                    r.claim_ids.iter().any(|v| has(v, x))
                        || searchable(r).iter().any(|(_, v)| has(v, x)),
                    "claim"
                )
            }
            let mut snippet = String::new();
            if let Some(x) = &q.search {
                let found = searchable(r)
                    .into_iter()
                    .filter(|(_, v)| has(v, x))
                    .collect::<Vec<_>>();
                if found.is_empty() {
                    return None;
                }
                for (name, _) in &found {
                    if !fields.iter().any(|f| f == name) {
                        fields.push((*name).into());
                    }
                }
                snippet = make_snippet(&found[0].1, x);
            }
            if q.min_score
                .is_some_and(|n| r.score_total.unwrap_or(i32::MIN) < n)
            {
                return None;
            }
            let scores = [
                r.s01_net, r.s02_net, r.s03_net, r.s04_net, r.s05_net, r.s06_net, r.s07_net,
                r.s08_net, r.s09_net, r.s10_net,
            ];
            if q.min_sections
                .iter()
                .zip(scores)
                .any(|(min, val)| min.is_some_and(|n| val.unwrap_or(i32::MIN) < n))
            {
                return None;
            }
            if q.min_evd_support
                .is_some_and(|n| r.evd_support.unwrap_or(i32::MIN) < n)
            {
                return None;
            }
            if let Some(want) = q.has_counter {
                if (r.evd_counter.unwrap_or(0) > 0) != want {
                    return None;
                }
            }
            Some(SearchResult {
                record: r.clone(),
                matched_fields: fields,
                snippet,
                index_name: index
                    .record_indexes
                    .get(&r.file_path)
                    .cloned()
                    .or_else(|| index.name.clone()),
            })
        })
        .collect();
    out.sort_by(|a, b| compare(&a.record, &b.record, &q.sort));
    out.truncate(q.limit);
    out
}
fn make_snippet(s: &str, q: &str) -> String {
    let lower = s.to_lowercase();
    let at = lower.find(&q.to_lowercase()).unwrap_or(0);
    let start = s[..at]
        .char_indices()
        .rev()
        .nth(40)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end = s[at..]
        .char_indices()
        .nth(100)
        .map(|(i, _)| at + i)
        .unwrap_or(s.len());
    s[start..end].replace('\n', " ")
}
fn compare(a: &IndexRecord, b: &IndexRecord, key: &str) -> Ordering {
    if key == "title" {
        return a
            .title
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b.title.as_deref().unwrap_or("").to_lowercase());
    }
    if key == "modified" {
        return b
            .file_modified
            .cmp(&a.file_modified)
            .then_with(|| a.file_path.cmp(&b.file_path));
    }
    let val = |r: &IndexRecord| match key {
        "s01" | "s01_net" => r.s01_net,
        "s02" | "s02_net" => r.s02_net,
        "s03" | "s03_net" => r.s03_net,
        "s04" | "s04_net" => r.s04_net,
        "s05" | "s05_net" => r.s05_net,
        "s06" | "s06_net" => r.s06_net,
        "s07" | "s07_net" => r.s07_net,
        "s08" | "s08_net" => r.s08_net,
        "s09" | "s09_net" => r.s09_net,
        "s10" | "s10_net" => r.s10_net,
        "evd_balance" => r.evd_balance,
        _ => r.score_total,
    };
    val(b)
        .unwrap_or(i32::MIN)
        .cmp(&val(a).unwrap_or(i32::MIN))
        .then_with(|| a.file_path.cmp(&b.file_path))
}
