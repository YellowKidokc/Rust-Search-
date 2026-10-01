use crate::model::{FileType, IndexRecord};
use anyhow::{Context, Result};
use quick_xml::{events::Event, Reader};
use std::{fs::File, io::Read, path::Path};
pub fn parse(path: &Path) -> Result<IndexRecord> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .context("DOCX has no word/document.xml")?
        .read_to_string(&mut xml)?;
    let mut reader = Reader::from_str(&xml);
    let mut text = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(t)) => text.push_str(&t.unescape()?),
            Ok(Event::End(e)) if e.name().as_ref() == b"w:p" => text.push_str("\n\n"),
            Ok(Event::Eof) => break,
            Err(e) => return Err(e.into()),
            _ => {}
        }
    }
    super::bare(path, FileType::Docx, text.trim().to_owned())
}
