use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileType {
    Markdown,
    Text,
    Json,
    Yaml,
    Pdf,
    Docx,
    Html,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TagWeight {
    pub tag: String,
    pub weight: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TruthPredicate {
    pub number: String,
    pub predicate: String,
    pub source_role: String,
    pub modality: String,
    pub formal_form: String,
    pub warrant: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Definition {
    pub term: String,
    pub plain_definition: String,
    pub domain: String,
    pub first_used_in: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexRecord {
    pub file_path: String,
    pub file_modified: u64,
    pub file_type: FileType,
    #[serde(default)]
    pub frontmatter: Value,
    pub title: Option<String>,
    pub clean_title: Option<String>,
    pub paper_id: Option<String>,
    pub paper_uuid: Option<String>,
    pub series: Option<String>,
    pub domain_primary: Option<String>,
    pub domain_secondary: Option<String>,
    pub domain_tertiary: Option<String>,
    pub content_type: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub tags_weighted: Vec<TagWeight>,
    #[serde(default)]
    pub topic_keys: Vec<String>,
    #[serde(default)]
    pub claim_ids: Vec<String>,
    pub governing_question: Option<String>,
    pub one_sentence_finding: Option<String>,
    pub score_total: Option<i32>,
    pub score_class: Option<String>,
    pub grade: Option<String>,
    pub s01_net: Option<i32>,
    pub s02_net: Option<i32>,
    pub s03_net: Option<i32>,
    pub s04_net: Option<i32>,
    pub s05_net: Option<i32>,
    pub s06_net: Option<i32>,
    pub s07_net: Option<i32>,
    pub s08_net: Option<i32>,
    pub s09_net: Option<i32>,
    pub s10_net: Option<i32>,
    pub evd_support: Option<i32>,
    pub evd_counter: Option<i32>,
    pub evd_balance: Option<i32>,
    pub evd_weakest_claim: Option<String>,
    pub coherence: Option<f64>,
    pub build_next: Option<String>,
    #[serde(default)]
    pub truth_predicates: Vec<TruthPredicate>,
    #[serde(default)]
    pub definitions: Vec<Definition>,
    pub body_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub roots: Vec<String>,
    pub records: Vec<IndexRecord>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub indexed_at: Option<String>,
    #[serde(default)]
    pub record_indexes: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub record: IndexRecord,
    pub matched_fields: Vec<String>,
    pub snippet: String,
    #[serde(default)]
    pub index_name: Option<String>,
}
