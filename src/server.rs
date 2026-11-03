use crate::{
    export,
    model::SearchResult,
    query::{self, Query},
    saved, store,
};
use anyhow::Result;
use axum::{
    extract::{Path, Query as AxumQuery, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    index_name: Option<String>,
    base: PathBuf,
}
#[derive(Deserialize, Default)]
pub struct ApiQuery {
    search: Option<String>,
    tag: Option<String>,
    topic: Option<String>,
    domain: Option<String>,
    series: Option<String>,
    claim: Option<String>,
    min_score: Option<i32>,
    min_s01: Option<i32>,
    min_s02: Option<i32>,
    min_s03: Option<i32>,
    min_s04: Option<i32>,
    min_s05: Option<i32>,
    min_s06: Option<i32>,
    min_s07: Option<i32>,
    min_s08: Option<i32>,
    min_s09: Option<i32>,
    min_s10: Option<i32>,
    min_evd_support: Option<i32>,
    has_counter: Option<bool>,
    sort: Option<String>,
    limit: Option<usize>,
}
impl ApiQuery {
    fn build(self) -> Query {
        let sections = [
            self.min_s01,
            self.min_s02,
            self.min_s03,
            self.min_s04,
            self.min_s05,
            self.min_s06,
            self.min_s07,
            self.min_s08,
            self.min_s09,
            self.min_s10,
        ];
        Query {
            search: self.search,
            tag: self.tag,
            topic: self.topic,
            domain: self.domain,
            series: self.series,
            claim: self.claim,
            min_score: self.min_score,
            min_sections: sections,
            min_evd_support: self.min_evd_support,
            has_counter: self.has_counter,
            sort: self.sort.unwrap_or_else(|| "score".into()),
            limit: self.limit.unwrap_or(100),
        }
    }
}
type ApiResult<T> = Result<T, (StatusCode, String)>;
fn err(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
fn load(s: &AppState) -> anyhow::Result<crate::model::Index> {
    match &s.index_name {
        Some(n) => store::load_named(n),
        None => store::load(&s.base),
    }
}
pub fn router(index_name: Option<String>, base: PathBuf) -> Router {
    Router::new()
        .route("/", get(|| async { Html(include_str!("web/index.html")) }))
        .route("/api/query", get(api_query))
        .route("/api/indexes", get(api_indexes))
        .route("/api/saved", get(list_saved))
        .route("/api/saved/:name", get(get_saved).post(save_search))
        .route("/api/export/files", post(files))
        .route("/api/export/csv", post(csv))
        .route("/api/export/manifest", post(manifest))
        .route("/api/export/snippets", post(snippets))
        .layer(CorsLayer::permissive())
        .with_state(Arc::new(AppState { index_name, base }))
}
pub async fn serve(port: u16, index_name: Option<String>, explicit: Option<PathBuf>) -> Result<()> {
    let base = match index_name.as_deref() {
        Some(n) if n != "all" => store::named_base(n)?,
        Some(_) => store::home()?,
        None => store::find_base(explicit.as_deref())?,
    };
    let app = router(index_name, base);
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("tpsearch web UI: http://{addr}");
    let _ = open::that(format!("http://{addr}"));
    axum::serve(listener, app).await?;
    Ok(())
}
async fn api_query(
    State(s): State<Arc<AppState>>,
    AxumQuery(q): AxumQuery<ApiQuery>,
) -> ApiResult<Json<Vec<SearchResult>>> {
    let i = load(&s).map_err(err)?;
    Ok(Json(query::execute(&i, &q.build())))
}
async fn api_indexes() -> ApiResult<Json<Vec<store::IndexInfo>>> {
    Ok(Json(store::list_named().map_err(err)?))
}
async fn list_saved(State(s): State<Arc<AppState>>) -> ApiResult<Json<Vec<String>>> {
    Ok(Json(saved::list(&s.base).map_err(err)?))
}
async fn get_saved(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> ApiResult<Json<saved::SavedSearch>> {
    Ok(Json(saved::load(&s.base, &name).map_err(err)?))
}
#[derive(Deserialize)]
struct SaveBody {
    query: Query,
    results: Vec<SearchResult>,
}
async fn save_search(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<SaveBody>,
) -> ApiResult<StatusCode> {
    saved::save(&s.base, &name, &body.query, &body.results).map_err(err)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct FilesBody {
    file_paths: Vec<String>,
    target: String,
}
async fn files(Json(body): Json<FilesBody>) -> ApiResult<StatusCode> {
    std::fs::create_dir_all(&body.target).map_err(err)?;
    for p in body.file_paths {
        let src = std::path::Path::new(&p);
        std::fs::copy(
            src,
            std::path::Path::new(&body.target).join(src.file_name().unwrap_or_default()),
        )
        .map_err(err)?;
    }
    Ok(StatusCode::NO_CONTENT)
}
fn download(content_type: &'static str, name: &str, data: Vec<u8>) -> impl IntoResponse {
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).unwrap(),
    );
    (h, data)
}
async fn csv(Json(results): Json<Vec<SearchResult>>) -> ApiResult<impl IntoResponse> {
    let f = temp_path("csv");
    export::csv(&results, &f).map_err(err)?;
    let b = std::fs::read(&f).map_err(err)?;
    let _ = std::fs::remove_file(f);
    Ok(download("text/csv", "tpsearch-results.csv", b))
}
async fn manifest(Json(results): Json<Vec<SearchResult>>) -> ApiResult<impl IntoResponse> {
    let b = serde_json::to_vec_pretty(&results).map_err(err)?;
    Ok(download("application/json", "tpsearch-manifest.json", b))
}
#[derive(Deserialize)]
struct SnippetBody {
    results: Vec<SearchResult>,
    query: Query,
}
async fn snippets(Json(body): Json<SnippetBody>) -> ApiResult<impl IntoResponse> {
    let text = export::snippets_markdown(&body.results, &body.query).map_err(err)?;
    Ok(download(
        "text/markdown",
        "tpsearch-snippets.md",
        text.into_bytes(),
    ))
}
fn temp_path(ext: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "tpsearch-{}-{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ext
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[tokio::test]
    async fn serves_embedded_frontend() {
        let dir = tempfile::tempdir().unwrap();
        let response = router(None, dir.path().to_owned())
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
