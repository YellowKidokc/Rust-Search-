//! Format adapters. Each turns a source file into passages with source
//! locations plus flat metadata. Originals are only ever read.
mod docx;
mod html;
mod pdf;
mod structured;
mod subtitles;
mod text;

use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Passage {
    pub text: String,
    /// 1-based. Lines for text formats, paragraphs for DOCX, cues for subtitles.
    pub loc_start: u32,
    pub loc_end: u32,
    pub section: Option<String>,
    pub t_start: Option<String>,
    pub t_end: Option<String>,
    pub page: Option<u32>,
}

#[derive(Debug, Default)]
pub struct Extracted {
    pub passages: Vec<Passage>,
    pub meta: Vec<(String, String)>,
    /// A source this file says it was derived from (JSON/YAML analysis output).
    pub derived_from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocKind {
    Line,
    Paragraph,
}
impl LocKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LocKind::Line => "line",
            LocKind::Paragraph => "paragraph",
        }
    }
}

pub struct Adapter {
    pub name: &'static str,
    /// Bump to force re-extraction of files handled by this adapter.
    pub version: i64,
    pub loc: LocKind,
    /// Original-file line ranges reproduce the passage text exactly.
    pub verifiable_by_lines: bool,
    run: fn(&[u8], &Path) -> Result<Extracted>,
}
impl Adapter {
    pub fn extract(&self, bytes: &[u8], path: &Path) -> Result<Extracted> {
        (self.run)(bytes, path)
    }
}

pub enum Support {
    Adapter(&'static Adapter),
    /// A planned adapter that this stage does not include.
    Pending(&'static str),
    /// Inventoried by name and metadata only.
    Unsupported,
}

static MARKDOWN: Adapter = Adapter {
    name: "markdown",
    version: 1,
    loc: LocKind::Line,
    verifiable_by_lines: true,
    run: text::markdown,
};
static PLAIN: Adapter = Adapter {
    name: "text",
    version: 1,
    loc: LocKind::Line,
    verifiable_by_lines: true,
    run: text::plain,
};
static JSON: Adapter = Adapter {
    name: "json",
    version: 1,
    loc: LocKind::Line,
    verifiable_by_lines: true,
    run: structured::json,
};
static YAML: Adapter = Adapter {
    name: "yaml",
    version: 1,
    loc: LocKind::Line,
    verifiable_by_lines: true,
    run: structured::yaml,
};
static SRT: Adapter = Adapter {
    name: "srt",
    version: 1,
    loc: LocKind::Line,
    verifiable_by_lines: true,
    run: subtitles::srt,
};
static VTT: Adapter = Adapter {
    name: "vtt",
    version: 1,
    loc: LocKind::Line,
    verifiable_by_lines: true,
    run: subtitles::vtt,
};
static HTML: Adapter = Adapter {
    name: "html",
    version: 1,
    loc: LocKind::Line,
    verifiable_by_lines: false,
    run: html::extract,
};
static DOCX: Adapter = Adapter {
    name: "docx",
    version: 1,
    loc: LocKind::Paragraph,
    verifiable_by_lines: false,
    run: docx::extract,
};

static PDF: Adapter = Adapter {
    name: "pdf",
    version: 1,
    loc: LocKind::Paragraph,
    verifiable_by_lines: false,
    run: pdf::extract,
};

pub fn support(ext: &str) -> Support {
    match ext {
        "md" | "markdown" => Support::Adapter(&MARKDOWN),
        "txt" | "text" | "log" | "csv" | "tsv" | "rst" | "org" | "tex" | "bib" | "ini" | "cfg"
        | "conf" | "toml" | "xml" | "rs" | "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "c"
        | "h" | "cpp" | "hpp" | "cs" | "java" | "kt" | "go" | "rb" | "php" | "swift" | "lua"
        | "r" | "jl" | "sql" | "sh" | "ps1" | "bat" | "cmd" | "css" | "scss" => {
            Support::Adapter(&PLAIN)
        }
        "json" => Support::Adapter(&JSON),
        "yaml" | "yml" => Support::Adapter(&YAML),
        "srt" => Support::Adapter(&SRT),
        "vtt" => Support::Adapter(&VTT),
        "html" | "htm" | "xhtml" => Support::Adapter(&HTML),
        "docx" => Support::Adapter(&DOCX),
        "pdf" => Support::Adapter(&PDF),
        _ => Support::Unsupported,
    }
}

pub fn adapter_by_name(name: &str) -> Option<&'static Adapter> {
    [&MARKDOWN, &PLAIN, &JSON, &YAML, &SRT, &VTT, &HTML, &DOCX, &PDF]
        .into_iter()
        .find(|a| a.name == name)
}

/// Decode text, honouring UTF-8/UTF-16 byte-order marks.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(b) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(b).into_owned();
    }
    let utf16 = |b: &[u8], le: bool| {
        let units: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(b) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(b, true);
    }
    if let Some(b) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(b, false);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Flatten a YAML/JSON value into `key = value` metadata pairs (two levels deep).
pub fn flatten_meta(
    v: &serde_yaml::Value,
    prefix: &str,
    out: &mut Vec<(String, String)>,
    depth: usize,
) {
    use serde_yaml::Value;
    let scalar = |v: &Value| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    };
    match v {
        Value::Mapping(m) if depth < 2 => {
            for (k, v) in m {
                let Some(k) = scalar(k) else { continue };
                let key = if prefix.is_empty() {
                    k
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_meta(v, &key, out, depth + 1);
            }
        }
        Value::Sequence(a) => {
            for x in a {
                match scalar(x) {
                    Some(s) => out.push((prefix.to_string(), s)),
                    None if depth < 2 => flatten_meta(x, prefix, out, depth + 1),
                    None => {}
                }
            }
        }
        Value::Tagged(t) => flatten_meta(&t.value, prefix, out, depth),
        other => {
            if let Some(s) = scalar(other) {
                if !prefix.is_empty() && s.len() <= 2000 {
                    out.push((prefix.to_string(), s));
                }
            }
        }
    }
}
