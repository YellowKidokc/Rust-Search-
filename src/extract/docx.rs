//! DOCX: paragraphs from word/document.xml; title/creator from docProps/core.xml.
//! Locations are 1-based paragraph numbers.
use super::{html::entities, Extracted, Passage};
use anyhow::{Context, Result};
use std::{
    io::{Cursor, Read},
    path::Path,
};

fn part(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Text of `<w:t>` runs (and tabs/breaks) in one paragraph's XML.
fn runs(p: &str) -> String {
    let mut out = String::new();
    let mut rest = p;
    while let Some(i) = rest.find('<') {
        let gt = match rest[i..].find('>') {
            Some(g) => i + g,
            None => break,
        };
        let tag = &rest[i + 1..gt];
        let name = tag
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('/');
        rest = &rest[gt + 1..];
        match name {
            "w:t" if !tag.ends_with('/') => {
                let end = rest.find("</w:t>").unwrap_or(rest.len());
                out.push_str(&entities(&rest[..end]));
                rest = &rest[end..];
            }
            "w:tab" => out.push('\t'),
            "w:br" | "w:cr" => out.push(' '),
            _ => {}
        }
    }
    out
}

pub(super) fn extract(bytes: &[u8], _: &Path) -> Result<Extracted> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).context("not a valid DOCX (zip) file")?;
    let doc = part(&mut zip, "word/document.xml").context("DOCX has no word/document.xml")?;
    let mut passages = vec![];
    let mut section = None;
    let mut n = 0u32;
    let mut rest = doc.as_str();
    while let Some(i) = rest.find("<w:p") {
        let after = &rest[i + 4..];
        // Only `<w:p>` / `<w:p ...>`, not `<w:pPr>`, `<w:pStyle>`, ...
        if !(after.starts_with('>') || after.starts_with(' ')) {
            rest = after;
            continue;
        }
        let end = after.find("</w:p>").unwrap_or(after.len());
        let body = &after[..end];
        rest = &after[end..];
        n += 1;
        let text = runs(body).trim().to_string();
        if text.is_empty() {
            continue;
        }
        let heading = body.contains("w:val=\"Heading") || body.contains("w:val=\"Title\"");
        if heading {
            section = Some(text.clone());
        }
        passages.push(Passage {
            text,
            loc_start: n,
            loc_end: n,
            section: section.clone(),
            ..Default::default()
        });
    }
    let mut meta = vec![];
    if let Some(core) = part(&mut zip, "docProps/core.xml") {
        for (tag, key) in [
            ("dc:title", "title"),
            ("dc:creator", "author"),
            ("dc:subject", "subject"),
            ("cp:keywords", "keywords"),
            ("dcterms:created", "created"),
        ] {
            if let Some(s) = core.find(&format!("<{tag}")) {
                let after = &core[s..];
                if let (Some(a), Some(b)) = (after.find('>'), after.find(&format!("</{tag}>"))) {
                    let v = entities(after[a + 1..b].trim());
                    if !v.is_empty() {
                        meta.push((key.to_string(), v));
                    }
                }
            }
        }
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
    use std::io::Write;
    #[test]
    fn docx_paragraphs() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("word/document.xml", o).unwrap();
            w.write_all(br#"<w:document><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Intro</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Hello </w:t></w:r><w:r><w:t>world &amp; more</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
            w.start_file("docProps/core.xml", o).unwrap();
            w.write_all(b"<cp:coreProperties><dc:title>Doc T</dc:title></cp:coreProperties>")
                .unwrap();
            w.finish().unwrap();
        }
        let e = extract(buf.get_ref(), Path::new("a.docx")).unwrap();
        assert_eq!(e.passages[1].text, "Hello world & more");
        assert_eq!(e.passages[1].loc_start, 2);
        assert_eq!(e.passages[1].section.as_deref(), Some("Intro"));
        assert_eq!(e.meta[0], ("title".into(), "Doc T".into()));
    }
}
