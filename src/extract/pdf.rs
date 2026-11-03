//! Text PDF extraction with page-local paragraph locations; OCR is deferred.
use super::{Extracted, Passage};
use anyhow::{Context, Result};
use pdf_extract::{Document, Object, PlainTextOutput};
use std::path::Path;

pub(super) fn extract(bytes: &[u8], _: &Path) -> Result<Extracted> {
    let mut doc = Document::load_mem(bytes).context("not a valid PDF")?;
    if doc.is_encrypted() {
        doc.decrypt(b"").context("PDF requires a password")?;
    }
    let mut pages = Vec::new();
    for number in doc.get_pages().keys() {
        let mut text = String::new();
        let mut output = PlainTextOutput::new(&mut text);
        pdf_extract::output_doc_page(&doc, &mut output, *number)
            .with_context(|| format!("extracting PDF page {number}"))?;
        pages.push((*number, text.replace("\r\n", "\n").replace('\r', "\n")));
    }
    if !pages.is_empty()
        && pages
            .iter()
            .map(|(_, s)| s.trim().chars().count())
            .sum::<usize>()
            < 100
    {
        return Ok(Extracted {
            meta: vec![("pdf_type".into(), "image_only".into())],
            ..Default::default()
        });
    }
    let mut result = Extracted {
        meta: vec![
            ("pdf_type".into(), "text".into()),
            ("page_count".into(), pages.len().to_string()),
        ],
        ..Default::default()
    };
    if let Ok(info) = doc.trailer.get(b"Info") {
        let (_, info) = doc.dereference(info)?;
        if let Ok(info) = info.as_dict() {
            for (pdf_key, key) in [
                (b"Title".as_slice(), "title"),
                (b"Author".as_slice(), "author"),
                (b"Subject".as_slice(), "subject"),
                (b"Keywords".as_slice(), "keywords"),
                (b"CreationDate".as_slice(), "created"),
            ] {
                if let Ok(value) = info.get(pdf_key) {
                    let (_, value) = doc.dereference(value)?;
                    if let Object::String(_, _) = value {
                        let value = pdf_extract::decode_text_string(value)?;
                        if !value.trim().is_empty() {
                            result.meta.push((key.into(), value));
                        }
                    }
                }
            }
        }
    }
    let mut section = None;
    for (page, text) in pages {
        for (index, paragraph) in text
            .split("\n\n")
            .filter(|s| !s.trim().is_empty())
            .enumerate()
        {
            let text = paragraph.trim();
            // Inspect standalone lines, retaining the most recent heading across pages.
            for line in text.lines() {
                if is_heading(line.trim()) {
                    section = Some(line.trim().to_owned());
                }
            }
            result.passages.push(Passage {
                text: text.into(),
                loc_start: (index + 1) as u32,
                loc_end: (index + 1) as u32,
                page: Some(page),
                section: section.clone(),
                ..Default::default()
            });
        }
    }
    Ok(result)
}

fn is_heading(line: &str) -> bool {
    let letters: Vec<_> = line.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() || line.chars().count() >= 120 {
        return false;
    }
    letters.iter().all(|c| c.is_uppercase())
        || (!line.ends_with(['.', '!', '?', ':', ';'])
            && line.split_whitespace().all(|word| {
                word.chars()
                    .find(|c| c.is_alphabetic())
                    .is_none_or(|c| c.is_uppercase())
            }))
}
