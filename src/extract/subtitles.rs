use super::{decode, Extracted, Passage};
use anyhow::Result;
use std::path::Path;

const CUES_PER_PASSAGE: usize = 6;

struct Cue {
    start: String,
    end: String,
    line_first: u32,
    line_last: u32,
    lines: Vec<String>,
}

/// Group cues into passages. Passage text is the cues' text lines (exact
/// source lines), joined by `\n`; locations are line ranges plus timestamps.
fn group(cues: Vec<Cue>) -> Vec<Passage> {
    cues.chunks(CUES_PER_PASSAGE)
        .map(|c| Passage {
            text: c
                .iter()
                .flat_map(|c| c.lines.iter().cloned())
                .collect::<Vec<_>>()
                .join("\n"),
            loc_start: c[0].line_first,
            loc_end: c[c.len() - 1].line_last,
            t_start: Some(c[0].start.clone()),
            t_end: Some(c[c.len() - 1].end.clone()),
            ..Default::default()
        })
        .collect()
}

fn parse(text: &str, vtt: bool) -> Vec<Cue> {
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut cues = vec![];
    let mut i = 0;
    while i < lines.len() {
        if let Some((a, b)) = lines[i].split_once("-->") {
            let clean = |s: &str| s.split_whitespace().next().unwrap_or("").replace(',', ".");
            let (start, end) = (clean(a), clean(b));
            let mut j = i + 1;
            let mut body = vec![];
            while j < lines.len() && !lines[j].trim().is_empty() {
                body.push(lines[j].to_string());
                j += 1;
            }
            if !body.is_empty() {
                cues.push(Cue {
                    start,
                    end,
                    line_first: i as u32 + 2,
                    line_last: j as u32,
                    lines: body,
                });
            }
            i = j;
        } else if vtt && (lines[i].starts_with("NOTE") || lines[i].starts_with("STYLE")) {
            while i < lines.len() && !lines[i].trim().is_empty() {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    cues
}

pub(super) fn srt(bytes: &[u8], _: &Path) -> Result<Extracted> {
    Ok(Extracted {
        passages: group(parse(&decode(bytes), false)),
        ..Default::default()
    })
}
pub(super) fn vtt(bytes: &[u8], _: &Path) -> Result<Extracted> {
    Ok(Extracted {
        passages: group(parse(&decode(bytes), true)),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn srt_timestamps_and_lines() {
        let s = "1\n00:00:01,000 --> 00:00:02,500\nHello there\n\n2\n00:00:03,000 --> 00:00:04,000\nSecond cue\nmore\n";
        let p = &srt(s.as_bytes(), Path::new("a.srt")).unwrap().passages[0];
        assert_eq!(p.t_start.as_deref(), Some("00:00:01.000"));
        assert_eq!(p.t_end.as_deref(), Some("00:00:04.000"));
        assert_eq!((p.loc_start, p.loc_end), (3, 8));
        assert!(p.text.contains("Second cue"));
        let v = "WEBVTT\n\nNOTE skip\n\n00:01.000 --> 00:02.000 align:start\nHi\n";
        let p = &vtt(v.as_bytes(), Path::new("a.vtt")).unwrap().passages[0];
        assert_eq!((p.text.as_str(), p.loc_start), ("Hi", 6));
    }
}
