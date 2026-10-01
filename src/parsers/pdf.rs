use crate::model::{FileType, IndexRecord};
use anyhow::Result;
use std::path::Path;
pub fn parse(path: &Path) -> Result<IndexRecord> {
    let text = pdf_extract::extract_text(path).unwrap_or_else(|e| {
        eprintln!(
            "warning: PDF text extraction failed for {}: {e}",
            path.display()
        );
        String::new()
    });
    super::bare(path, FileType::Pdf, text)
}
