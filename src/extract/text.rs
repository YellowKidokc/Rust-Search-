use super::{decode, flatten_meta, Extracted, Passage};
use crate::parsers::markdown::split_frontmatter;
use anyhow::Result;
use std::path::Path;

const MAX_PASSAGE: usize = 1500;

/// Split lines into passages at blank lines, headings, and a size cap.
/// Passage text is the exact original lines joined by `\n`, so it can be
/// re-verified against the source file.
pub(super) fn split_lines(text: &str, first_line: u32, markdown: bool) -> Vec<Passage> {
    let mut out = vec![];
    let mut cur: Vec<&str> = vec![];
    let mut start = 0u32;
    let mut len = 0usize;
    let mut section: Option<String> = None;
    let mut fence = false;
    let flush = |cur: &mut Vec<&str>,
                 start: u32,
                 len: &mut usize,
                 section: &Option<String>,
                 out: &mut Vec<Passage>| {
        if cur.iter().any(|l| !l.trim().is_empty()) {
            out.push(Passage {
                text: cur.join("\n"),
                loc_start: start,
                loc_end: start + cur.len() as u32 - 1,
                section: section.clone(),
                ..Default::default()
            });
        }
        cur.clear();
        *len = 0;
    };
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let n = first_line + i as u32;
        if markdown && line.trim_start().starts_with("```") {
            fence = !fence;
        }
        let heading = markdown
            && !fence
            && line.starts_with('#')
            && line.trim_start_matches('#').starts_with(' ');
        if line.trim().is_empty() || heading || len + line.len() > MAX_PASSAGE {
            flush(&mut cur, start, &mut len, &section, &mut out);
        }
        if heading {
            section = Some(line.trim_start_matches('#').trim().to_string());
        }
        if line.trim().is_empty() {
            continue;
        }
        if cur.is_empty() {
            start = n;
        }
        cur.push(line);
        len += line.len() + 1;
        if heading {
            flush(&mut cur, start, &mut len, &section, &mut out);
        }
    }
    flush(&mut cur, start, &mut len, &section, &mut out);
    out
}

pub(super) fn plain(bytes: &[u8], _: &Path) -> Result<Extracted> {
    Ok(Extracted {
        passages: split_lines(&decode(bytes), 1, false),
        ..Default::default()
    })
}

pub(super) fn markdown(bytes: &[u8], _: &Path) -> Result<Extracted> {
    let text = decode(bytes);
    let (fm, body) = split_frontmatter(&text);
    let body_line = 1 + text[..text.len() - body.len()].matches('\n').count() as u32;
    let mut meta = vec![];
    if !fm.is_empty() {
        match serde_yaml::from_str::<serde_yaml::Value>(fm) {
            Ok(v) => flatten_meta(&v, "", &mut meta, 0),
            Err(e) => meta.push(("_frontmatter_error".into(), e.to_string())),
        }
    }
    Ok(Extracted {
        passages: split_lines(body, body_line, true),
        meta,
        derived_from: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_lines_and_sections() {
        let s = "---\ntitle: T\n---\n# Intro\nalpha line\nbeta\n\n## Next\n```\n# not heading\n```\nend\n";
        let e = markdown(s.as_bytes(), Path::new("x.md")).unwrap();
        assert_eq!(e.meta, vec![("title".into(), "T".into())]);
        let p = &e.passages;
        assert_eq!(p[0].text, "# Intro");
        assert_eq!(p[1].text, "alpha line\nbeta");
        assert_eq!((p[1].loc_start, p[1].loc_end), (5, 6));
        assert_eq!(p[1].section.as_deref(), Some("Intro"));
        let last = p.last().unwrap();
        assert_eq!(last.section.as_deref(), Some("Next"));
        assert!(last.text.contains("# not heading"));
        let lines: Vec<&str> = s.lines().collect();
        for x in p {
            assert_eq!(
                lines[x.loc_start as usize - 1..x.loc_end as usize].join("\n"),
                x.text
            );
        }
    }
}
