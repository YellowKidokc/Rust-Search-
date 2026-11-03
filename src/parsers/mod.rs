pub mod docx;
pub mod html;
pub mod json;
pub mod markdown;
pub mod pdf;
pub mod text;
pub mod yaml;

use crate::model::{FileType, IndexRecord};
use anyhow::Result;
use serde_yaml::{Mapping, Value};
use std::{fs, path::Path, time::UNIX_EPOCH};

pub(crate) fn bare(path: &Path, file_type: FileType, body_text: String) -> Result<IndexRecord> {
    let modified = fs::metadata(path)?
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(IndexRecord {
        file_path: path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned(),
        file_modified: modified,
        file_type,
        frontmatter: Value::Mapping(Mapping::new()),
        title: None,
        clean_title: None,
        paper_id: None,
        paper_uuid: None,
        series: None,
        domain_primary: None,
        domain_secondary: None,
        domain_tertiary: None,
        content_type: None,
        tags: vec![],
        tags_weighted: vec![],
        topic_keys: vec![],
        claim_ids: vec![],
        governing_question: None,
        one_sentence_finding: None,
        score_total: None,
        score_class: None,
        grade: None,
        s01_net: None,
        s02_net: None,
        s03_net: None,
        s04_net: None,
        s05_net: None,
        s06_net: None,
        s07_net: None,
        s08_net: None,
        s09_net: None,
        s10_net: None,
        evd_support: None,
        evd_counter: None,
        evd_balance: None,
        evd_weakest_claim: None,
        coherence: None,
        build_next: None,
        truth_predicates: vec![],
        definitions: vec![],
        body_text,
    })
}

pub(crate) fn structured(
    path: &Path,
    kind: FileType,
    yaml: Value,
    body: String,
) -> Result<IndexRecord> {
    let mut r = bare(path, kind, body)?;
    r.frontmatter = yaml.clone();
    macro_rules! string {
        ($field:ident) => {
            r.$field = value_string(&yaml, stringify!($field));
        };
    }
    macro_rules! int {
        ($field:ident) => {
            r.$field = value_i32(&yaml, stringify!($field));
        };
    }
    string!(title);
    string!(clean_title);
    string!(paper_id);
    string!(paper_uuid);
    string!(series);
    string!(domain_primary);
    string!(domain_secondary);
    string!(domain_tertiary);
    string!(content_type);
    string!(governing_question);
    string!(one_sentence_finding);
    string!(score_class);
    string!(grade);
    string!(evd_weakest_claim);
    string!(build_next);
    int!(score_total);
    int!(s01_net);
    int!(s02_net);
    int!(s03_net);
    int!(s04_net);
    int!(s05_net);
    int!(s06_net);
    int!(s07_net);
    int!(s08_net);
    int!(s09_net);
    int!(s10_net);
    int!(evd_support);
    int!(evd_counter);
    int!(evd_balance);
    r.coherence =
        get(&yaml, "coherence").and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()));
    r.tags = values(&yaml, "tags");
    r.topic_keys = values(&yaml, "topic_keys");
    r.claim_ids = values(&yaml, "claim_ids");
    r.tags_weighted = typed(&yaml, "tags_weighted");
    r.truth_predicates = typed(&yaml, "truth_predicates");
    r.definitions = typed(&yaml, "definitions");
    Ok(r)
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
fn value_string(v: &Value, key: &str) -> Option<String> {
    get(v, key).and_then(scalar)
}
fn value_i32(v: &Value, key: &str) -> Option<i32> {
    get(v, key).and_then(|v| {
        v.as_i64()
            .map(|n| n as i32)
            .or_else(|| v.as_str()?.parse().ok())
    })
}
fn values(v: &Value, key: &str) -> Vec<String> {
    get(v, key)
        .map(|v| match v {
            Value::Sequence(xs) => xs.iter().filter_map(scalar).collect(),
            _ => scalar(v).into_iter().collect(),
        })
        .unwrap_or_default()
}
fn typed<T: serde::de::DeserializeOwned>(v: &Value, key: &str) -> Vec<T> {
    get(v, key)
        .and_then(|value| serde_yaml::from_value(value.clone()).ok())
        .unwrap_or_default()
}
