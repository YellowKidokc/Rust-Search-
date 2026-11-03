//! Dependency-free HTML text extraction. Locations are the source line where
//! each block's text begins; text is decoded, so verification uses the stored
//! extraction plus the source hash rather than raw lines.
use super::{decode, Extracted, Passage};
use anyhow::Result;
use std::path::Path;

const BLOCKS: [&str; 22] = [
    "p",
    "div",
    "br",
    "li",
    "tr",
    "td",
    "th",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "section",
    "article",
    "header",
    "footer",
    "blockquote",
    "pre",
    "table",
    "ul",
    "ol",
];

pub fn entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(12)].find(';');
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub(super) fn extract(bytes: &[u8], _: &Path) -> Result<Extracted> {
    let src = decode(bytes);
    let mut passages = vec![];
    let mut meta = vec![];
    let mut buf = String::new();
    let mut buf_line = 0u32;
    let mut line = 1u32;
    let mut section: Option<String> = None;
    let mut in_heading = false;
    let mut in_title = false;
    let mut title = String::new();
    let mut skip_until: Option<&str> = None;
    let mut rest = src.as_str();
    let flush = |buf: &mut String,
                 start: u32,
                 end: u32,
                 section: &Option<String>,
                 out: &mut Vec<Passage>| {
        let t = entities(&buf.split_whitespace().collect::<Vec<_>>().join(" "));
        if !t.is_empty() {
            out.push(Passage {
                text: t,
                loc_start: start,
                loc_end: end.max(start),
                section: section.clone(),
                ..Default::default()
            });
        }
        buf.clear();
    };
    while !rest.is_empty() {
        if let Some(close) = skip_until {
            match rest.to_ascii_lowercase().find(close) {
                Some(i) => {
                    line += rest[..i].matches('\n').count() as u32;
                    rest = &rest[i + close.len()..];
                }
                None => break,
            }
            skip_until = None;
            continue;
        }
        let Some(lt) = rest.find('<') else {
            if buf.is_empty() {
                buf_line = line;
            }
            buf.push_str(rest);
            break;
        };
        let txt = &rest[..lt];
        if !txt.trim().is_empty() && buf.trim().is_empty() {
            let lead = &txt[..txt.len() - txt.trim_start().len()];
            buf_line = line + lead.matches('\n').count() as u32;
        }
        if in_title {
            title.push_str(txt);
        } else {
            buf.push_str(txt);
        }
        line += txt.matches('\n').count() as u32;
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            skip_until = Some("-->");
            rest = &rest[4..];
            continue;
        }
        let gt = rest.find('>').unwrap_or(rest.len() - 1);
        let tag = &rest[1..gt];
        line += tag.matches('\n').count() as u32;
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match name.as_str() {
            "script" | "style" | "noscript" | "svg" if !closing => {
                skip_until = Some(match name.as_str() {
                    "script" => "</script>",
                    "style" => "</style>",
                    "noscript" => "</noscript>",
                    _ => "</svg>",
                });
                continue;
            }
            "title" => {
                in_title = !closing;
                continue;
            }
            _ => {}
        }
        if BLOCKS.contains(&name.as_str()) {
            let is_h =
                name.len() == 2 && name.starts_with('h') && name.as_bytes()[1].is_ascii_digit();
            if is_h && closing && in_heading {
                let h = entities(&buf.split_whitespace().collect::<Vec<_>>().join(" "));
                if !h.is_empty() {
                    section = Some(h);
                }
            }
            flush(&mut buf, buf_line, line, &section, &mut passages);
            if is_h {
                in_heading = !closing;
            }
        }
    }
    flush(&mut buf, buf_line, line, &section, &mut passages);
    let title = entities(title.trim());
    if !title.is_empty() {
        meta.push(("title".into(), title));
    }
    Ok(Extracted {
        passages,
        meta,
        derived_from: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_blocks() {
        let s = "<html><head><title>T &amp; U</title><style>p{}</style></head>\n<body>\n<h1>Head</h1>\n<p>Alpha &lt;b&gt;\nbeta</p><script>var x='<p>no</p>';</script><p>Gamma</p></body>";
        let e = extract(s.as_bytes(), Path::new("a.html")).unwrap();
        assert_eq!(e.meta[0].1, "T & U");
        let texts: Vec<_> = e.passages.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, ["Head", "Alpha <b> beta", "Gamma"]);
        assert_eq!(e.passages[1].loc_start, 4);
        assert_eq!(e.passages[1].section.as_deref(), Some("Head"));
    }
}
