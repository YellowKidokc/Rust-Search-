//! `tpsearch serve`: a loopback-only HTTP front end for the search UI.
//! One search core (`search::run`) behind every UI mode; modes only change
//! the filters they send and how they display/export the results.
use crate::{config, db, report, search};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

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

#[derive(Clone)]
struct State {
    config: std::path::PathBuf,
    default_collection: Option<String>,
    jobs: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, Value>>>,
}

type ApiResult =
    std::result::Result<axum::Json<Value>, (axum::http::StatusCode, axum::Json<Value>)>;
fn response(r: Result<Value>) -> ApiResult {
    r.map(axum::Json).map_err(|e| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": format!("{e:#}")})),
        )
    })
}

pub async fn run(
    cfg_path: &Path,
    port: u16,
    open_browser: bool,
    default_collection: Option<String>,
) -> Result<()> {
    let cfg = config::load(cfg_path)?;
    if let Some(name) = &default_collection {
        cfg.get(name)?;
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    println!("tpsearch UI at http://127.0.0.1:{port}/ (Ctrl+C to stop)");
    if open_browser {
        open::that(format!("http://127.0.0.1:{port}/"))?;
    }
    let state = State {
        config: cfg_path.into(),
        default_collection,
        jobs: Default::default(),
    };
    axum::serve(listener, router(state)).await?;
    Ok(())
}

fn router(state: State) -> axum::Router {
    use axum::{
        extract::{Query, State as S},
        response::Html,
        routing::{get, post},
        Json,
    };
    axum::Router::new()
        .route("/", get(|| async { Html(UI) }))
        .route("/api/collections", get(|S(s): S<State>| async move { response(collections(&s.config)) }))
        .route("/api/config", get(|S(s): S<State>| async move { Json(json!({"default_collection":s.default_collection})) }))
        .route("/api/search", post(|S(s): S<State>, Json(q): Json<SearchReq>| async move {
            response(do_search(&s.config, q))
        }).get(|S(s): S<State>, Query(p): Query<std::collections::HashMap<String,String>>| async move {
            response(search_params(&s, p))
        }))
        .route("/api/status", get(|S(s): S<State>, Query(p): Query<std::collections::HashMap<String,String>>| async move {
            response((|| {
                let cfg = config::load(&s.config)?;
                let name = selected(&s, &p)?;
                let dir = config::index_dir(&name, cfg.get(&name)?)?;
                let mut value = serde_json::to_value(report::status(&name, &dir)?)?;
                let conn = db::open(&dir)?;
                let mut counts = conn.prepare("SELECT ext, COUNT(*) FROM files GROUP BY ext ORDER BY ext")?;
                let by_type = counts.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
                    .collect::<std::result::Result<std::collections::BTreeMap<_,_>,_>>()?;
                value["files_by_type"] = json!(by_type);
                value["pdf_needs_ocr"] = json!(conn.query_row("SELECT COUNT(DISTINCT file_id) FROM meta WHERE key='pdf_type' AND value='image_only'", [], |r| r.get::<_, i64>(0))?);
                Ok(value)
            })())
        }))
        .route("/api/index", post(|S(s): S<State>, Query(p): Query<std::collections::HashMap<String,String>>| async move {
            response(start_index(s, p))
        }).get(|S(s): S<State>, Query(p): Query<std::collections::HashMap<String,String>>| async move {
            response((|| { let name = selected(&s, &p)?; Ok(s.jobs.lock().unwrap().get(&name).cloned().unwrap_or(json!({"collection":name,"status":"idle"}))) })())
        }))
        .route("/api/open", post(|S(s): S<State>, Json(p): Json<OpenReq>| async move {
            response(crate::cli::open(&s.config, &p.collection, p.passage_id, false).map(|_| json!({"opened":true})))
        }))
        .route("/api/folder-report", post(|S(s): S<State>, Json(q): Json<FolderReq>| async move {
            response(folder_report(&s.config, q))
        }))
        .layer(axum::middleware::from_fn(loopback_request))
        .with_state(state)
}

async fn loopback_request(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let local_host = host == "localhost"
        || host == "127.0.0.1"
        || ["localhost:", "127.0.0.1:"].iter().any(|prefix| {
            host.strip_prefix(prefix)
                .is_some_and(|port| port.parse::<u16>().is_ok())
        });
    let same_origin = request.headers().get("origin").is_none_or(|origin| {
        origin
            .to_str()
            .is_ok_and(|origin| origin == format!("http://{host}"))
    });
    if !local_host || !same_origin {
        return (
            axum::http::StatusCode::FORBIDDEN,
            axum::Json(json!({"error":"only same-origin loopback requests are accepted"})),
        )
            .into_response();
    }
    next.run(request).await
}

#[derive(Deserialize)]
struct OpenReq {
    collection: String,
    passage_id: i64,
}

fn selected(s: &State, p: &std::collections::HashMap<String, String>) -> Result<String> {
    p.get("collection")
        .filter(|n| !n.is_empty())
        .cloned()
        .or_else(|| s.default_collection.clone())
        .context("choose a collection")
}

fn search_params(s: &State, p: std::collections::HashMap<String, String>) -> Result<Value> {
    let flag = |k: &str| -> Result<bool> {
        match p.get(k).map(String::as_str) {
            None | Some("") | Some("false") => Ok(false),
            Some("true") => Ok(true),
            _ => anyhow::bail!("{k} expects true or false"),
        }
    };
    let list = |k: &str| {
        p.get(k)
            .map(|x| {
                x.split(',')
                    .filter(|x| !x.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let mut names = list("collection");
    if !p.contains_key("collection") {
        if let Some(n) = &s.default_collection {
            names.push(n.clone());
        }
    }
    let q = SearchReq {
        collections: names,
        query: p.get("q").cloned().unwrap_or_default(),
        phrase: flag("phrase")?,
        any: flag("any")?,
        types: list("type"),
        path: p.get("path").cloned(),
        under: p.get("under").cloned(),
        after: p.get("after").cloned(),
        before: p.get("before").cloned(),
        meta: p.get("meta").map(|m| vec![m.clone()]).unwrap_or_default(),
        representation: p.get("representation").cloned(),
        limit: p.get("limit").map(|n| n.parse()).transpose()?,
        include_missing: flag("include_missing")?,
        no_verify: flag("no_verify")?,
    };
    do_search(&s.config, q)
}

fn start_index(s: State, p: std::collections::HashMap<String, String>) -> Result<Value> {
    let name = selected(&s, &p)?;
    let cfg = config::load(&s.config)?;
    let collection = cfg.get(&name)?.clone();
    let dir = config::index_dir(&name, &collection)?;
    let dry_run = match p.get("dry_run").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        _ => anyhow::bail!("dry_run expects true or false"),
    };
    let running = json!({"collection":name,"status":"running","dry_run":dry_run});
    {
        let mut jobs = s.jobs.lock().unwrap();
        anyhow::ensure!(
            jobs.get(&name).is_none_or(|j| j["status"] != "running"),
            "index job already running for {name}"
        );
        jobs.insert(name.clone(), running.clone());
    }
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value> {
            if dry_run {
                let inv = crate::scan::inventory(&collection, &dir)?;
                let diff = crate::scan::diff(&inv, &dir)?;
                Ok(json!({"inventory": inv, "new_changed_unchanged": diff}))
            } else {
                Ok(serde_json::to_value(crate::scan::run(&collection, &dir)?)?)
            }
        }))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("index worker panicked")));
        let job = match result {
            Ok(stats) => {
                json!({"collection":name,"status":"completed","dry_run":dry_run,"result":stats})
            }
            Err(e) => json!({"collection":name,"status":"failed","error":format!("{e:#}")}),
        };
        s.jobs.lock().unwrap().insert(name, job);
    });
    Ok(running)
}

fn collections(cfg_path: &Path) -> Result<Value> {
    let cfg = config::load(cfg_path)?;
    let mut v = vec![];
    for (name, c) in &cfg.collections {
        let dir = config::index_dir(name, c)?;
        v.push(json!({
            "name": name,
            "indexed": db::exists(&dir),
            "status": if db::exists(&dir) { Some(serde_json::to_value(report::status(name, &dir)?)?) } else { None },
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
    anyhow::ensure!(
        !all.is_empty(),
        "none of the selected collections are indexed yet"
    );
    Ok(json!(all))
}

fn folder_report(cfg_path: &Path, q: FolderReq) -> Result<Value> {
    let cfg = config::load(cfg_path)?;
    let dir = config::index_dir(&q.collection, cfg.get(&q.collection)?)?;
    report::folder(&q.collection, &dir, &report::resolve_folder(&q.folder))
}
