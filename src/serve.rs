//! `tpsearch serve`: a loopback-only HTTP front end for the search UI.
//! One search core (`search::run`) behind every UI mode; modes only change
//! the filters they send and how they display/export the results.
use crate::{config, db, report, search};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
};

const UI: &str = include_str!("../ui/search.html");

#[derive(Deserialize, Default)]
#[serde(default)]
struct SearchReq {
    collections: Vec<String>,
    query: String,
    phrase: bool,
    any: bool,
    types: Vec<String>,
    path: Option<String>,
    under: Option<String>,
    after: Option<String>,
    before: Option<String>,
    meta: Vec<String>,
    representation: Option<String>,
    limit: Option<usize>,
    include_missing: bool,
    no_verify: bool,
}

#[derive(Deserialize)]
struct FolderReq {
    collection: String,
    folder: String,
}

fn blank(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.trim().is_empty())
}

pub fn run(cfg_path: &Path, port: u16, open: bool) -> Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr).with_context(|| format!("binding {addr}"))?;
    let url = format!("http://{addr}/");
    println!("tpsearch UI at {url}  (Ctrl+C to stop)");
    if open {
        #[cfg(windows)]
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn();
        #[cfg(not(windows))]
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }
    for stream in listener.incoming() {
        let Ok(s) = stream else { continue };
        if let Err(e) = handle(cfg_path, s) {
            eprintln!("request error: {e:#}");
        }
    }
    Ok(())
}

fn handle(cfg_path: &Path, mut s: TcpStream) -> Result<()> {
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; len.min(1 << 20)];
    r.read_exact(&mut body)?;
    let path = target.split('?').next().unwrap_or("/");

    let (status, ctype, out) = match (method.as_str(), path) {
        ("GET", "/") => ("200 OK", "text/html; charset=utf-8", UI.to_string()),
        _ => {
            let res = match (method.as_str(), path) {
                ("GET", "/api/collections") => collections(cfg_path),
                ("POST", "/api/search") => serde_json::from_slice(&body)
                    .map_err(Into::into)
                    .and_then(|q| do_search(cfg_path, q)),
                ("POST", "/api/folder-report") => serde_json::from_slice(&body)
                    .map_err(Into::into)
                    .and_then(|q| folder_report(cfg_path, q)),
                _ => Err(anyhow::anyhow!("not found: {method} {path}")),
            };
            match res {
                Ok(v) => ("200 OK", "application/json", v.to_string()),
                Err(e) => (
                    "400 Bad Request",
                    "application/json",
                    json!({ "error": format!("{e:#}") }).to_string(),
                ),
            }
        }
    };
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{out}",
        out.len()
    )?;
    Ok(())
}

fn collections(cfg_path: &Path) -> Result<Value> {
    let cfg = config::load(cfg_path)?;
    let mut v = vec![];
    for (name, c) in &cfg.collections {
        let dir = config::index_dir(name, c)?;
        v.push(json!({
            "name": name,
            "indexed": db::exists(&dir),
            "roots": c.roots.iter().map(|r| r.path.display().to_string()).collect::<Vec<_>>(),
        }));
    }
    Ok(json!(v))
}

fn do_search(cfg_path: &Path, q: SearchReq) -> Result<Value> {
    let cfg = config::load(cfg_path)?;
    let names: Vec<String> = if q.collections.is_empty() {
        cfg.collections.keys().cloned().collect()
    } else {
        q.collections.clone()
    };
    anyhow::ensure!(!names.is_empty(), "no collections configured");
    anyhow::ensure!(!q.query.trim().is_empty(), "enter a query");
    let opts = search::Opts {
        query: q.query.clone(),
        phrase: q.phrase,
        any: q.any,
        ext: q.types.iter().filter(|t| !t.is_empty()).cloned().collect(),
        path: blank(q.path),
        under: blank(q.under).as_deref().map(report::resolve_folder),
        after: blank(q.after).as_deref().map(db::parse_date).transpose()?,
        before: blank(q.before).as_deref().map(db::parse_date).transpose()?,
        meta: q
            .meta
            .iter()
            .filter(|m| !m.trim().is_empty())
            .map(|m| {
                m.split_once('=')
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .context("metadata filter expects key=value")
            })
            .collect::<Result<_>>()?,
        representation: blank(q.representation),
        limit: q.limit.unwrap_or(20).clamp(1, 500),
        include_missing: q.include_missing,
        verify: !q.no_verify,
    };
    let mut all = vec![];
    for n in &names {
        let c = cfg.get(n)?;
        let dir = config::index_dir(n, c)?;
        if !db::exists(&dir) {
            continue;
        }
        all.push(serde_json::to_value(search::run(n, &dir, &opts)?)?);
    }
    anyhow::ensure!(!all.is_empty(), "none of the selected collections are indexed yet");
    Ok(json!(all))
}

fn folder_report(cfg_path: &Path, q: FolderReq) -> Result<Value> {
    let cfg = config::load(cfg_path)?;
    let dir = config::index_dir(&q.collection, cfg.get(&q.collection)?)?;
    report::folder(&q.collection, &dir, &report::resolve_folder(&q.folder))
}
