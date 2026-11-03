#[allow(dead_code)]
#[path = "../src/parsers/markdown.rs"]
pub mod markdown;
#[allow(dead_code)]
#[path = "../src/model.rs"]
mod model;
mod parsers {
    pub use crate::markdown;
}
// Exercise the production adapters directly: this binary crate has no library target.
#[allow(dead_code)]
#[path = "../src/extract/mod.rs"]
mod extract;
use extract::{support, Support};
use std::path::Path;

fn extract_pdf(bytes: &[u8]) -> extract::Extracted {
    let Support::Adapter(adapter) = support("pdf") else {
        panic!("PDF adapter missing")
    };
    assert_eq!(adapter.name, "pdf");
    assert!(!adapter.verifiable_by_lines);
    assert_eq!(extract::adapter_by_name("pdf").unwrap().version, 1);
    adapter.extract(bytes, Path::new("sample.pdf")).unwrap()
}

#[test]
fn text_pdf_has_pages_passages_and_metadata() {
    let e = extract_pdf(include_bytes!("fixtures/sample.pdf"));
    for page in [1, 2] {
        assert!(e.passages.iter().any(|p| p.page == Some(page)));
    }
    assert!(e
        .passages
        .iter()
        .all(|p| p.page.is_some() && p.loc_start > 0 && p.loc_start == p.loc_end));
    assert!(e
        .passages
        .iter()
        .any(|p| p.text.contains("quality of mercy")));
    assert!(e
        .passages
        .iter()
        .any(|p| p.section.as_deref() == Some("FIRST CHAPTER")));
    for (key, value) in [
        ("pdf_type", "text"),
        ("page_count", "2"),
        ("author", "William Shakespeare"),
        ("title", "Public Domain Shakespeare"),
        ("subject", "Sample"),
        ("keywords", "mercy stage"),
        ("created", "D:20260101000000Z"),
    ] {
        assert!(
            e.meta.contains(&(key.into(), value.into())),
            "{key}: {:?}",
            e.meta
        );
    }
}

#[test]
fn image_pdf_needs_ocr_without_an_error() {
    let e = extract_pdf(include_bytes!("fixtures/image.pdf"));
    assert!(e.passages.is_empty());
    assert_eq!(e.meta, vec![("pdf_type".into(), "image_only".into())]);
}

#[test]
fn invalid_pdf_is_an_error() {
    let Support::Adapter(adapter) = support("pdf") else {
        panic!()
    };
    assert!(adapter
        .extract(b"%PDF-1.4 fake", Path::new("broken.pdf"))
        .is_err());
}
